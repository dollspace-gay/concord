use anyhow::{Context, Result, anyhow};
use atproto_identity::key::KeyData;
use atproto_oauth::dpop::{auth_dpop, request_dpop};
use atproto_oauth::jwk;
use atproto_oauth::jwt::{self, Claims, Header, JoseClaims};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::db::queries::users;

fn account_refresh_lock(user_id: &str) -> Result<std::sync::Arc<tokio::sync::Mutex<()>>> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, Weak};
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow!("credential refresh coordinator unavailable"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(user_id).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    if locks.len() >= 4096 {
        return Err(anyhow!("credential refresh coordinator is busy"));
    }
    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(user_id.to_owned(), std::sync::Arc::downgrade(&lock));
    Ok(lock)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PdsRequestError {
    #[error("PDS authentication was rejected")]
    Authentication,
    #[error("PDS returned status {status}")]
    RemoteStatus {
        status: reqwest::StatusCode,
        body: Vec<u8>,
    },
    #[error("PDS request outcome is uncertain")]
    Uncertain(#[source] crate::egress::EgressError),
    #[error("PDS request could not be constructed")]
    Local,
}

struct DpopRequest<'a> {
    transport: &'a crate::egress::ControlledHttpClient,
    key: &'a KeyData,
    access_token: &'a str,
    method: &'a str,
    url: &'a str,
    body: Option<&'a [u8]>,
    content_type: &'a str,
}

/// Blob reference returned by the PDS after upload.
#[derive(Debug, Clone)]
pub struct BlobRef {
    /// Content Identifier (CID) of the blob.
    pub cid: String,
    /// URL to download the blob from the PDS.
    pub url: String,
    /// MIME type as stored by the PDS (may differ from what was uploaded).
    pub mime_type: Option<String>,
}

#[derive(Deserialize)]
struct UploadBlobResponse {
    blob: BlobData,
}

#[derive(Deserialize)]
struct BlobData {
    #[serde(rename = "$type")]
    _type: Option<String>,
    #[serde(rename = "ref")]
    ref_link: Option<RefLink>,
    #[serde(rename = "cid")]
    cid_str: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Deserialize)]
struct RefLink {
    #[serde(rename = "$link")]
    link: String,
}

// ── Generic Authenticated XRPC Caller ──────────────────────────────────────

/// Parameters for an authenticated XRPC call against a user's PDS.
pub struct PdsXrpcParams<'a> {
    pub session: &'a PdsSession<'a>,
    /// HTTP method: "GET" or "POST".
    pub method: &'a str,
    /// XRPC method name (e.g., "com.atproto.repo.createRecord").
    pub xrpc_endpoint: &'a str,
    /// Request body (None for GET requests).
    pub body: Option<&'a [u8]>,
    /// Content-Type header (e.g., "application/json").
    pub content_type: &'a str,
}

pub struct PdsSession<'a> {
    pub transport: &'a crate::egress::ControlledHttpClient,
    pub pool: &'a SqlitePool,
    pub vault: &'a crate::secrets::SecretVault,
    pub user_id: &'a str,
    pub signing_key: &'a KeyData,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
}

/// Perform an authenticated XRPC request against a user's PDS.
///
/// Handles DPoP proof generation, nonce challenges, and automatic token refresh.
/// Returns the raw response body as bytes on success.
pub async fn pds_xrpc_call(p: &PdsXrpcParams<'_>) -> Result<Vec<u8>> {
    let s = p.session;
    let creds = users::get_atproto_credentials_encrypted(s.pool, s.vault, s.user_id)
        .await
        .context("DB error fetching AT Protocol credentials")?
        .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;

    let dpop_key = deserialize_dpop_key(&creds.dpop_private_key)?;
    let url = format!("{}/xrpc/{}", creds.pds_url, p.xrpc_endpoint);

    // Try the request, refreshing token once if it fails
    match do_dpop_request(&DpopRequest {
        transport: s.transport,
        key: &dpop_key,
        access_token: &creds.access_token,
        method: p.method,
        url: &url,
        body: p.body,
        content_type: p.content_type,
    })
    .await
    {
        Ok(bytes) => Ok(bytes),
        Err(PdsRequestError::Authentication) => {
            warn!("PDS rejected access token; attempting token refresh");
            let refreshed = refresh_access_token(s, &creds, &dpop_key).await?;
            let refreshed_key = deserialize_dpop_key(&refreshed.dpop_private_key)?;
            let refreshed_url = format!("{}/xrpc/{}", refreshed.pds_url, p.xrpc_endpoint);
            do_dpop_request(&DpopRequest {
                transport: s.transport,
                key: &refreshed_key,
                access_token: &refreshed.access_token,
                method: p.method,
                url: &refreshed_url,
                body: p.body,
                content_type: p.content_type,
            })
            .await
            .context("PDS XRPC call failed after token refresh")
        }
        Err(error) => Err(error.into()),
    }
}

async fn do_dpop_request(request: &DpopRequest<'_>) -> Result<Vec<u8>, PdsRequestError> {
    let (proof, _, _) = request_dpop(
        request.key,
        request.method,
        request.url,
        request.access_token,
    )
    .map_err(|_| PdsRequestError::Local)?;
    let response = send_dpop(request, &proof).await?;
    if response.status == reqwest::StatusCode::UNAUTHORIZED
        && let Some(nonce) = response
            .headers
            .get("DPoP-Nonce")
            .and_then(|value| value.to_str().ok())
    {
        return do_dpop_request_with_nonce(request, nonce).await;
    }
    if response.status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(PdsRequestError::Authentication);
    }
    if !response.status.is_success() {
        return Err(PdsRequestError::RemoteStatus {
            status: response.status,
            body: response.body,
        });
    }
    Ok(response.body)
}

async fn do_dpop_request_with_nonce(
    request: &DpopRequest<'_>,
    nonce: &str,
) -> Result<Vec<u8>, PdsRequestError> {
    let (_, header, mut claims) = request_dpop(
        request.key,
        request.method,
        request.url,
        request.access_token,
    )
    .map_err(|_| PdsRequestError::Local)?;
    claims
        .private
        .insert("nonce".into(), nonce.to_string().into());
    let proof = jwt::mint(request.key, &header, &claims).map_err(|_| PdsRequestError::Local)?;
    let response = send_dpop(request, &proof).await?;
    if response.status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(PdsRequestError::Authentication);
    }
    if !response.status.is_success() {
        return Err(PdsRequestError::RemoteStatus {
            status: response.status,
            body: response.body,
        });
    }
    Ok(response.body)
}

async fn send_dpop(
    dpop: &DpopRequest<'_>,
    proof: &str,
) -> Result<crate::egress::EgressResponse, PdsRequestError> {
    let method = match dpop.method {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        _ => return Err(PdsRequestError::Local),
    };
    let url = reqwest::Url::parse(dpop.url).map_err(|_| PdsRequestError::Local)?;
    let authorization =
        reqwest::header::HeaderValue::from_str(&format!("DPoP {}", dpop.access_token))
            .map_err(|_| PdsRequestError::Local)?;
    let proof =
        reqwest::header::HeaderValue::from_str(proof).map_err(|_| PdsRequestError::Local)?;
    let mut request = dpop
        .transport
        .request(method, url.clone(), crate::egress::RedirectPolicy::Reject)
        .map_err(|_| PdsRequestError::Local)?
        .header(reqwest::header::AUTHORIZATION, authorization)
        .header(reqwest::header::HeaderName::from_static("dpop"), proof)
        .credentials_for(&url)
        .map_err(|_| PdsRequestError::Local)?;
    if let Some(body) = dpop.body {
        request = request
            .header(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_str(dpop.content_type)
                    .map_err(|_| PdsRequestError::Local)?,
            )
            .body(body.to_vec());
    }
    dpop.transport
        .send(request)
        .await
        .map_err(PdsRequestError::Uncertain)
}

/// Deserialize a DPoP private key from stored JWK JSON.
fn deserialize_dpop_key(dpop_private_key_json: &str) -> Result<KeyData> {
    let wrapped_jwk: jwk::WrappedJsonWebKey = serde_json::from_str(dpop_private_key_json)
        .context("Failed to deserialize stored DPoP key from JWK")?;
    jwk::to_key_data(&wrapped_jwk).map_err(|e| anyhow!("Invalid stored DPoP JWK: {e:?}"))
}

// ── Blob Upload (uses generic XRPC caller internally) ──────────────────────

/// Upload a blob to the user's PDS using their stored AT Protocol credentials.
/// Returns the blob CID and download URL.
///
/// `signing_key` is the server's persistent signing key for client assertions.
/// `client_id` and `redirect_uri` are the OAuth client metadata values.
pub async fn upload_blob_to_pds(
    session: &PdsSession<'_>,
    file_bytes: Vec<u8>,
    content_type: &str,
) -> Result<BlobRef> {
    let creds =
        users::get_atproto_credentials_encrypted(session.pool, session.vault, session.user_id)
            .await
            .context("DB error fetching AT Protocol credentials")?
            .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;

    let mut active = creds;
    let mut dpop_key = deserialize_dpop_key(&active.dpop_private_key)?;
    let mut upload_url = format!("{}/xrpc/com.atproto.repo.uploadBlob", active.pds_url);

    // Try upload, refreshing token once if expired
    let blob_resp = match do_upload(
        session.transport,
        &dpop_key,
        &active.access_token,
        &upload_url,
        &file_bytes,
        content_type,
    )
    .await
    {
        Ok(resp) => resp,
        Err(PdsRequestError::Authentication) => {
            warn!("PDS rejected blob upload access token; attempting token refresh");
            active = refresh_access_token(session, &active, &dpop_key).await?;
            dpop_key = deserialize_dpop_key(&active.dpop_private_key)?;
            upload_url = format!("{}/xrpc/com.atproto.repo.uploadBlob", active.pds_url);
            do_upload(
                session.transport,
                &dpop_key,
                &active.access_token,
                &upload_url,
                &file_bytes,
                content_type,
            )
            .await
            .context("PDS upload failed after token refresh")?
        }
        Err(error) => return Err(error.into()),
    };

    let blob_ref = finalize_blob_ref(&blob_resp, &active.pds_url, &active.did);

    // Pin the blob by creating a record that references it in the user's repo.
    let pin_mime_type = blob_ref.mime_type.as_deref().unwrap_or(content_type);
    let file_size = file_bytes.len();
    if let Err(e) = pin_blob_with_record(&PinBlobRequest {
        transport: session.transport,
        dpop_key: &dpop_key,
        access_token: &active.access_token,
        pds_url: &active.pds_url,
        did: &active.did,
        cid: &blob_ref.cid,
        content_type: pin_mime_type,
        file_size,
    })
    .await
    {
        warn!(error = %e, "Failed to pin blob with createRecord (blob may not be servable)");
    }

    Ok(blob_ref)
}

/// Perform the actual blob upload with DPoP auth.
async fn do_upload(
    transport: &crate::egress::ControlledHttpClient,
    dpop_key: &KeyData,
    access_token: &str,
    upload_url: &str,
    file_bytes: &[u8],
    content_type: &str,
) -> Result<UploadBlobResponse, PdsRequestError> {
    let bytes = do_dpop_request(&DpopRequest {
        transport,
        key: dpop_key,
        access_token,
        method: "POST",
        url: upload_url,
        body: Some(file_bytes),
        content_type,
    })
    .await?;
    crate::egress::parse_provider_json(&bytes).map_err(|_| PdsRequestError::Local)
}

fn finalize_blob_ref(resp: &UploadBlobResponse, pds_url: &str, did: &str) -> BlobRef {
    let cid = resp
        .blob
        .ref_link
        .as_ref()
        .map(|r| r.link.clone())
        .or_else(|| resp.blob.cid_str.clone())
        .unwrap_or_default();

    let url = format!(
        "{}/xrpc/com.atproto.sync.getBlob?did={}&cid={}",
        pds_url,
        urlencoding::encode(did),
        urlencoding::encode(&cid)
    );

    BlobRef {
        cid,
        url,
        mime_type: resp.blob.mime_type.clone(),
    }
}

// ── Record Creation (for blob pinning and future custom lexicons) ──────────

/// JSON body for com.atproto.repo.createRecord
#[derive(Serialize)]
pub struct CreateRecordRequest<T: Serialize> {
    pub repo: String,
    pub collection: String,
    pub record: T,
}

/// A minimal record that references a blob, pinning it in the user's PDS repo.
#[derive(Serialize)]
struct AttachmentRecord {
    #[serde(rename = "$type")]
    record_type: String,
    blob: BlobObject,
    #[serde(rename = "createdAt")]
    created_at: String,
}

/// AT Protocol blob reference object for embedding in records.
#[derive(Serialize)]
struct BlobObject {
    #[serde(rename = "$type")]
    blob_type: String,
    #[serde(rename = "ref")]
    ref_link: BlobLink,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: usize,
}

#[derive(Serialize)]
struct BlobLink {
    #[serde(rename = "$link")]
    link: String,
}

struct PinBlobRequest<'a> {
    transport: &'a crate::egress::ControlledHttpClient,
    dpop_key: &'a KeyData,
    access_token: &'a str,
    pds_url: &'a str,
    did: &'a str,
    cid: &'a str,
    content_type: &'a str,
    file_size: usize,
}

/// Create a record in the user's PDS repo that references the uploaded blob.
/// This pins the blob so it can be served via com.atproto.sync.getBlob.
async fn pin_blob_with_record(request: &PinBlobRequest<'_>) -> Result<()> {
    let create_url = format!("{}/xrpc/com.atproto.repo.createRecord", request.pds_url);

    let body = CreateRecordRequest {
        repo: request.did.to_string(),
        collection: "chat.concord.attachment".to_string(),
        record: AttachmentRecord {
            record_type: "chat.concord.attachment".to_string(),
            blob: BlobObject {
                blob_type: "blob".to_string(),
                ref_link: BlobLink {
                    link: request.cid.to_string(),
                },
                mime_type: request.content_type.to_string(),
                size: request.file_size,
            },
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    };

    let body_json =
        serde_json::to_string(&body).context("Failed to serialize createRecord body")?;

    do_dpop_request(&DpopRequest {
        transport: request.transport,
        key: request.dpop_key,
        access_token: request.access_token,
        method: "POST",
        url: &create_url,
        body: Some(body_json.as_bytes()),
        content_type: "application/json",
    })
    .await?;

    Ok(())
}

/// Create any record in a user's PDS repo using the generic XRPC caller.
/// This is the high-level helper for creating records with automatic token refresh.
pub async fn create_record<T: Serialize>(
    session: &PdsSession<'_>,
    collection: &str,
    record: &T,
) -> Result<CreateRecordResponse> {
    let creds =
        users::get_atproto_credentials_encrypted(session.pool, session.vault, session.user_id)
            .await
            .context("DB error fetching AT Protocol credentials")?
            .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;

    let body = serde_json::json!({
        "repo": creds.did,
        "collection": collection,
        "record": record,
    });
    let body_json =
        serde_json::to_string(&body).context("Failed to serialize createRecord body")?;

    let resp_bytes = pds_xrpc_call(&PdsXrpcParams {
        session,
        method: "POST",
        xrpc_endpoint: "com.atproto.repo.createRecord",
        body: Some(body_json.as_bytes()),
        content_type: "application/json",
    })
    .await?;

    crate::egress::parse_provider_json(&resp_bytes).map_err(Into::into)
}

/// Create or replace a record at a deterministic repository key. Repeating a
/// request after an uncertain response addresses the same AT record identity.
pub async fn put_record<T: Serialize>(
    session: &PdsSession<'_>,
    collection: &str,
    record_key: &str,
    record: &T,
) -> Result<CreateRecordResponse> {
    if record_key.is_empty()
        || !record_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'~'))
    {
        return Err(anyhow!("Invalid AT Protocol record key"));
    }
    let creds =
        users::get_atproto_credentials_encrypted(session.pool, session.vault, session.user_id)
            .await?
            .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;
    let body = serde_json::json!({
        "repo": creds.did,
        "collection": collection,
        "rkey": record_key,
        "record": record,
    });
    let body_json = serde_json::to_string(&body)?;
    let bytes = pds_xrpc_call(&PdsXrpcParams {
        session,
        method: "POST",
        xrpc_endpoint: "com.atproto.repo.putRecord",
        body: Some(body_json.as_bytes()),
        content_type: "application/json",
    })
    .await?;
    crate::egress::parse_provider_json(&bytes).map_err(Into::into)
}

/// Response from com.atproto.repo.createRecord
#[derive(Debug, Clone, Deserialize)]
pub struct CreateRecordResponse {
    pub uri: String,
    pub cid: String,
}

// ── Token Refresh ──────────────────────────────────────────────────────────

/// Build a private_key_jwt client assertion for the given token endpoint.
fn build_client_assertion(signing_key: &KeyData, client_id: &str, issuer: &str) -> Result<String> {
    let header: Header = signing_key
        .clone()
        .try_into()
        .map_err(|e| anyhow!("Failed to create client assertion header: {e:?}"))?;

    let jti = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp() as u64;

    let claims = Claims::new(JoseClaims {
        issuer: Some(client_id.to_string()),
        subject: Some(client_id.to_string()),
        audience: Some(issuer.to_string()),
        json_web_token_id: Some(jti),
        issued_at: Some(now),
        ..Default::default()
    });

    jwt::mint(signing_key, &header, &claims)
        .map_err(|e| anyhow!("Failed to mint client assertion JWT: {e}"))
}

/// Refresh the AT Protocol access token using the stored refresh token.
#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u32,
}

async fn refresh_access_token(
    session: &PdsSession<'_>,
    creds: &users::AtprotoCredentials,
    dpop_key: &KeyData,
) -> Result<users::AtprotoCredentials> {
    let account_lock = account_refresh_lock(session.user_id)?;
    let _guard = account_lock.lock().await;
    let current =
        users::get_atproto_credentials_encrypted(session.pool, session.vault, session.user_id)
            .await?
            .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;
    if current.credential_version != creds.credential_version {
        return Ok(current);
    }
    let creds = &current;
    if creds.refresh_token.is_empty() {
        return Err(anyhow!("No refresh token available"));
    }
    if creds.authorization_issuer.is_empty() || creds.token_endpoint.is_empty() {
        return Err(anyhow!(
            "Stored authorization grant requires reauthentication"
        ));
    }
    let issuer = reqwest::Url::parse(&creds.authorization_issuer)
        .map_err(|_| anyhow!("Invalid stored authorization issuer"))?;
    let token_url = reqwest::Url::parse(&creds.token_endpoint)
        .map_err(|_| anyhow!("Invalid token endpoint"))?;
    if token_url.origin() != issuer.origin() {
        return Err(anyhow!("Token endpoint origin mismatch"));
    }
    let assertion = build_client_assertion(
        session.signing_key,
        session.client_id,
        &creds.authorization_issuer,
    )?;
    let (proof, _, _) = auth_dpop(dpop_key, "POST", token_url.as_str())
        .context("Failed to create refresh DPoP proof")?;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", session.client_id)
        .append_pair("redirect_uri", session.redirect_uri)
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", &creds.refresh_token)
        .append_pair(
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
        )
        .append_pair("client_assertion", &assertion)
        .finish();
    let mut response =
        send_token_request(session.transport, &token_url, &proof, form.as_bytes()).await?;
    if matches!(
        response.status,
        reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNAUTHORIZED
    ) && let Some(nonce) = response
        .headers
        .get("DPoP-Nonce")
        .and_then(|value| value.to_str().ok())
    {
        let (_, header, mut claims) = auth_dpop(dpop_key, "POST", token_url.as_str())
            .context("Failed to create nonce refresh proof")?;
        claims
            .private
            .insert("nonce".into(), nonce.to_string().into());
        let nonce_proof = jwt::mint(dpop_key, &header, &claims)
            .map_err(|_| anyhow!("Failed to mint nonce refresh proof"))?;
        response = send_token_request(session.transport, &token_url, &nonce_proof, form.as_bytes())
            .await?;
    }
    if !response.status.is_success() {
        return Err(anyhow!("Token refresh returned status {}", response.status));
    }
    let refresh: RefreshResponse = crate::egress::parse_provider_json(&response.body)?;
    let updated = users::AtprotoCredentials {
        did: creds.did.clone(),
        access_token: refresh.access_token.clone(),
        refresh_token: refresh
            .refresh_token
            .unwrap_or_else(|| creds.refresh_token.clone()),
        dpop_private_key: creds.dpop_private_key.clone(),
        pds_url: creds.pds_url.clone(),
        authorization_issuer: creds.authorization_issuer.clone(),
        token_endpoint: creds.token_endpoint.clone(),
        token_expires_at: (chrono::Utc::now()
            + chrono::Duration::seconds(refresh.expires_in as i64))
        .to_rfc3339(),
        credential_version: creds.credential_version,
    };
    let stored = users::store_atproto_credentials_if_version(
        session.pool,
        session.vault,
        session.user_id,
        creds.credential_version,
        &updated,
    )
    .await?;
    let current =
        users::get_atproto_credentials_encrypted(session.pool, session.vault, session.user_id)
            .await?
            .ok_or_else(|| anyhow!("AT Protocol credential disappeared after refresh"))?;
    if stored {
        info!(user_id=%session.user_id,"AT Protocol tokens refreshed");
    } else {
        info!(user_id=%session.user_id,"AT Protocol reauthentication superseded token refresh");
    }
    Ok(current)
}

async fn send_token_request(
    transport: &crate::egress::ControlledHttpClient,
    url: &reqwest::Url,
    proof: &str,
    body: &[u8],
) -> Result<crate::egress::EgressResponse> {
    let request = transport
        .request(
            reqwest::Method::POST,
            url.clone(),
            crate::egress::RedirectPolicy::Reject,
        )?
        .header(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .header(
            reqwest::header::HeaderName::from_static("dpop"),
            reqwest::header::HeaderValue::from_str(proof)
                .map_err(|_| anyhow!("Invalid DPoP proof"))?,
        )
        .body(body.to_vec())
        .credentials_for(url)?;
    transport.send(request).await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atproto_identity::key::{KeyType, generate_key};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn accepted_mutation_with_lost_response_is_uncertain_and_not_authentication() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = std::sync::Arc::new(AtomicUsize::new(0));
        let observed = accepted.clone();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let read = socket.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST "));
            observed.fetch_add(1, Ordering::SeqCst);
            // The remote accepted the mutation and then lost the response.
        });
        let transport = crate::egress::ControlledHttpClient::fixture(address, 1024);
        let key = generate_key(KeyType::P256Private).unwrap();
        let result = do_dpop_request(&DpopRequest {
            transport: &transport,
            key: &key,
            access_token: "access-token",
            method: "POST",
            url: "http://fixture.test/xrpc/com.atproto.repo.createRecord",
            body: Some(br#"{"repo":"did:example:alice"}"#),
            content_type: "application/json",
        })
        .await;
        assert!(matches!(result, Err(PdsRequestError::Uncertain(_))));
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_coordinator_serializes_the_same_account() {
        let first = account_refresh_lock("did:example:alice").unwrap();
        let second = account_refresh_lock("did:example:alice").unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        let held = first.lock().await;
        let waiting = tokio::spawn(async move {
            let _guard = second.lock().await;
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(held);
        waiting.await.unwrap();
    }

    #[tokio::test]
    async fn refresh_uses_bound_origin_and_yields_to_concurrent_reauthentication() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0_u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap();
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with("POST /bound-token HTTP/1.1"));
            request_seen_tx.send(()).unwrap();
            respond_rx.await.unwrap();
            let body = r#"{"access_token":"stale-refresh-access","refresh_token":"stale-refresh-token","expires_in":3600}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        users::create_with_oauth(
            &pool,
            &users::CreateOAuthUser {
                user_id: "alice",
                username: "alice",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-alice",
                provider: "atproto",
                provider_id: "did:example:alice",
            },
        )
        .await
        .unwrap();
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([37_u8; 32])).unwrap();
        let vault = crate::secrets::SecretVault::load(key_file.path()).unwrap();
        let old_dpop = generate_key(KeyType::P256Private).unwrap();
        let old_dpop_json = serde_json::to_string(&jwk::generate(&old_dpop).unwrap()).unwrap();
        let old = users::AtprotoCredentials {
            did: "did:example:alice".into(),
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            dpop_private_key: old_dpop_json,
            pds_url: "https://old-pds.example".into(),
            authorization_issuer: "http://fixture.test".into(),
            token_endpoint: "http://fixture.test/bound-token".into(),
            token_expires_at: "2026-01-01T00:00:00Z".into(),
            credential_version: 0,
        };
        users::store_atproto_credentials_encrypted(&pool, &vault, "alice", &old)
            .await
            .unwrap();
        let old = users::get_atproto_credentials_encrypted(&pool, &vault, "alice")
            .await
            .unwrap()
            .unwrap();
        let transport = crate::egress::ControlledHttpClient::fixture(address, 4096);
        let signing_key = generate_key(KeyType::P256Private).unwrap();
        let session = PdsSession {
            transport: &transport,
            pool: &pool,
            vault: &vault,
            user_id: "alice",
            signing_key: &signing_key,
            client_id: "https://concord.example/oauth/client-metadata.json",
            redirect_uri: "https://concord.example/oauth/atproto/callback",
        };
        let refreshed = refresh_access_token(&session, &old, &old_dpop);
        let replace = async {
            request_seen_rx.await.unwrap();
            let new_dpop = generate_key(KeyType::P256Private).unwrap();
            let replacement = users::AtprotoCredentials {
                did: "did:example:alice".into(),
                access_token: "reauth-access".into(),
                refresh_token: "reauth-refresh".into(),
                dpop_private_key: serde_json::to_string(&jwk::generate(&new_dpop).unwrap())
                    .unwrap(),
                pds_url: "https://new-pds.example".into(),
                authorization_issuer: "https://new-issuer.example".into(),
                token_endpoint: "https://new-issuer.example/token".into(),
                token_expires_at: "2027-01-01T00:00:00Z".into(),
                credential_version: 0,
            };
            users::store_atproto_credentials_encrypted(&pool, &vault, "alice", &replacement)
                .await
                .unwrap();
            respond_tx.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(refreshed, replace);
        let result = result.unwrap();
        assert_eq!(result.access_token, "reauth-access");
        assert_eq!(result.refresh_token, "reauth-refresh");
        assert_eq!(result.pds_url, "https://new-pds.example");
        assert_eq!(result.authorization_issuer, "https://new-issuer.example");
        assert_eq!(result.token_endpoint, "https://new-issuer.example/token");
        assert_ne!(result.dpop_private_key, old.dpop_private_key);
        server.await.unwrap();
    }
}
