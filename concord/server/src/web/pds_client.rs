use anyhow::{Context, Result, anyhow};
use atproto_identity::key::KeyData;
use atproto_oauth::dpop::{auth_dpop, request_dpop};
use atproto_oauth::jwk;
use atproto_oauth::jwt::{self, Claims, Header, JoseClaims};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::db::queries::users;

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
    pub pool: &'a SqlitePool,
    pub user_id: &'a str,
    /// HTTP method: "GET" or "POST".
    pub method: &'a str,
    /// XRPC method name (e.g., "com.atproto.repo.createRecord").
    pub xrpc_endpoint: &'a str,
    /// Request body (None for GET requests).
    pub body: Option<&'a [u8]>,
    /// Content-Type header (e.g., "application/json").
    pub content_type: &'a str,
    pub signing_key: &'a KeyData,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
}

/// Perform an authenticated XRPC request against a user's PDS.
///
/// Handles DPoP proof generation, nonce challenges, and automatic token refresh.
/// Returns the raw response body as bytes on success.
pub async fn pds_xrpc_call(p: &PdsXrpcParams<'_>) -> Result<Vec<u8>> {
    let creds = users::get_atproto_credentials(p.pool, p.user_id)
        .await
        .context("DB error fetching AT Protocol credentials")?
        .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;

    let dpop_key = deserialize_dpop_key(&creds.dpop_private_key)?;
    let url = format!("{}/xrpc/{}", creds.pds_url, p.xrpc_endpoint);

    // Try the request, refreshing token once if it fails
    match do_dpop_request(
        &dpop_key,
        &creds.access_token,
        p.method,
        &url,
        p.body,
        p.content_type,
    )
    .await
    {
        Ok(bytes) => Ok(bytes),
        Err(e) => {
            warn!(error = %e, "PDS XRPC call failed, attempting token refresh");
            let new_token = refresh_access_token(
                p.pool,
                p.user_id,
                &creds,
                &dpop_key,
                p.signing_key,
                p.client_id,
                p.redirect_uri,
            )
            .await?;
            do_dpop_request(
                &dpop_key,
                &new_token,
                p.method,
                &url,
                p.body,
                p.content_type,
            )
            .await
            .context("PDS XRPC call failed after token refresh")
        }
    }
}

/// Perform an HTTP request with DPoP authentication, handling nonce challenges.
async fn do_dpop_request(
    dpop_key: &KeyData,
    access_token: &str,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    content_type: &str,
) -> Result<Vec<u8>> {
    let (dpop_token, _header, _claims) =
        request_dpop(dpop_key, method, url, access_token).context("Failed to create DPoP proof")?;

    let http_client = reqwest::Client::new();

    let mut req = match method {
        "GET" => http_client.get(url),
        "POST" => http_client.post(url),
        _ => return Err(anyhow!("Unsupported HTTP method: {}", method)),
    };

    req = req
        .header("Authorization", format!("DPoP {}", access_token))
        .header("DPoP", &dpop_token);

    if let Some(body_bytes) = body {
        req = req
            .header("Content-Type", content_type)
            .body(body_bytes.to_vec());
    }

    let resp = req.send().await.context("HTTP request to PDS failed")?;

    // Handle DPoP nonce challenge: if server returns 401 with DPoP-Nonce, retry
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED
        && let Some(nonce) = resp
            .headers()
            .get("DPoP-Nonce")
            .and_then(|v| v.to_str().ok())
    {
        return do_dpop_request_with_nonce(
            dpop_key,
            access_token,
            method,
            url,
            body,
            content_type,
            nonce,
        )
        .await;
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "PDS XRPC {} returned {}: {}",
            url,
            status,
            body_text
        ));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .context("Failed to read PDS response body")
}

/// Retry a DPoP request with a nonce (after server challenge).
async fn do_dpop_request_with_nonce(
    dpop_key: &KeyData,
    access_token: &str,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    content_type: &str,
    nonce: &str,
) -> Result<Vec<u8>> {
    let (_dpop_token, header, mut claims) = request_dpop(dpop_key, method, url, access_token)
        .context("Failed to create DPoP proof for nonce retry")?;

    claims
        .private
        .insert("nonce".to_string(), nonce.to_string().into());
    let dpop_token_with_nonce = jwt::mint(dpop_key, &header, &claims)
        .map_err(|e| anyhow!("Failed to mint DPoP with nonce: {e}"))?;

    let http_client = reqwest::Client::new();

    let mut req = match method {
        "GET" => http_client.get(url),
        "POST" => http_client.post(url),
        _ => return Err(anyhow!("Unsupported HTTP method: {}", method)),
    };

    req = req
        .header("Authorization", format!("DPoP {}", access_token))
        .header("DPoP", &dpop_token_with_nonce);

    if let Some(body_bytes) = body {
        req = req
            .header("Content-Type", content_type)
            .body(body_bytes.to_vec());
    }

    let resp = req
        .send()
        .await
        .context("HTTP request to PDS failed (nonce retry)")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "PDS XRPC {} returned {} (nonce retry): {}",
            url,
            status,
            body_text
        ));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .context("Failed to read PDS response body (nonce retry)")
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
    pool: &SqlitePool,
    user_id: &str,
    file_bytes: Vec<u8>,
    content_type: &str,
    signing_key: &KeyData,
    client_id: &str,
    redirect_uri: &str,
) -> Result<BlobRef> {
    let creds = users::get_atproto_credentials(pool, user_id)
        .await
        .context("DB error fetching AT Protocol credentials")?
        .ok_or_else(|| anyhow!("No AT Protocol credentials for user"))?;

    let dpop_key = deserialize_dpop_key(&creds.dpop_private_key)?;
    let pds_url = &creds.pds_url;
    let upload_url = format!("{}/xrpc/com.atproto.repo.uploadBlob", pds_url);

    // Try upload, refreshing token once if expired
    let (blob_resp, token_used) = match do_upload(
        &dpop_key,
        &creds.access_token,
        &upload_url,
        &file_bytes,
        content_type,
    )
    .await
    {
        Ok(resp) => (resp, creds.access_token.clone()),
        Err(e) => {
            warn!(error = %e, "PDS upload failed, attempting token refresh");
            let new_token = refresh_access_token(
                pool,
                user_id,
                &creds,
                &dpop_key,
                signing_key,
                client_id,
                redirect_uri,
            )
            .await?;
            let resp = do_upload(
                &dpop_key,
                &new_token,
                &upload_url,
                &file_bytes,
                content_type,
            )
            .await
            .context("PDS upload failed after token refresh")?;
            (resp, new_token)
        }
    };

    let blob_ref = finalize_blob_ref(&blob_resp, pds_url, &creds.did);

    // Pin the blob by creating a record that references it in the user's repo.
    let pin_mime_type = blob_ref.mime_type.as_deref().unwrap_or(content_type);
    let file_size = file_bytes.len();
    if let Err(e) = pin_blob_with_record(
        &dpop_key,
        &token_used,
        pds_url,
        &creds.did,
        &blob_ref.cid,
        pin_mime_type,
        file_size,
    )
    .await
    {
        warn!(error = %e, "Failed to pin blob with createRecord (blob may not be servable)");
    }

    Ok(blob_ref)
}

/// Perform the actual blob upload with DPoP auth.
async fn do_upload(
    dpop_key: &KeyData,
    access_token: &str,
    upload_url: &str,
    file_bytes: &[u8],
    content_type: &str,
) -> Result<UploadBlobResponse> {
    let bytes = do_dpop_request(
        dpop_key,
        access_token,
        "POST",
        upload_url,
        Some(file_bytes),
        content_type,
    )
    .await?;
    serde_json::from_slice(&bytes).context("Failed to parse PDS upload response")
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

/// Create a record in the user's PDS repo that references the uploaded blob.
/// This pins the blob so it can be served via com.atproto.sync.getBlob.
async fn pin_blob_with_record(
    dpop_key: &KeyData,
    access_token: &str,
    pds_url: &str,
    did: &str,
    cid: &str,
    content_type: &str,
    file_size: usize,
) -> Result<()> {
    let create_url = format!("{}/xrpc/com.atproto.repo.createRecord", pds_url);

    let body = CreateRecordRequest {
        repo: did.to_string(),
        collection: "chat.concord.attachment".to_string(),
        record: AttachmentRecord {
            record_type: "chat.concord.attachment".to_string(),
            blob: BlobObject {
                blob_type: "blob".to_string(),
                ref_link: BlobLink {
                    link: cid.to_string(),
                },
                mime_type: content_type.to_string(),
                size: file_size,
            },
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    };

    let body_json =
        serde_json::to_string(&body).context("Failed to serialize createRecord body")?;

    do_dpop_request(
        dpop_key,
        access_token,
        "POST",
        &create_url,
        Some(body_json.as_bytes()),
        "application/json",
    )
    .await?;

    Ok(())
}

/// Create any record in a user's PDS repo using the generic XRPC caller.
/// This is the high-level helper for creating records with automatic token refresh.
pub async fn create_record<T: Serialize>(
    pool: &SqlitePool,
    user_id: &str,
    collection: &str,
    record: &T,
    signing_key: &KeyData,
    client_id: &str,
    redirect_uri: &str,
) -> Result<CreateRecordResponse> {
    let creds = users::get_atproto_credentials(pool, user_id)
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
        pool,
        user_id,
        method: "POST",
        xrpc_endpoint: "com.atproto.repo.createRecord",
        body: Some(body_json.as_bytes()),
        content_type: "application/json",
        signing_key,
        client_id,
        redirect_uri,
    })
    .await?;

    serde_json::from_slice(&resp_bytes).context("Failed to parse createRecord response")
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
async fn refresh_access_token(
    pool: &SqlitePool,
    user_id: &str,
    creds: &users::AtprotoCredentials,
    dpop_key: &KeyData,
    signing_key: &KeyData,
    client_id: &str,
    redirect_uri: &str,
) -> Result<String> {
    if creds.refresh_token.is_empty() {
        return Err(anyhow!("No refresh token available"));
    }

    let http_client = reqwest::Client::new();

    // Discover the authorization server from the PDS
    let (_resource, auth_server) =
        atproto_oauth::resources::pds_resources(&http_client, &creds.pds_url)
            .await
            .context("Failed to discover PDS auth server for token refresh")?;

    let token_endpoint = &auth_server.token_endpoint;

    // Build client assertion (private_key_jwt) signed with the server's signing key
    let client_assertion = build_client_assertion(signing_key, client_id, &auth_server.issuer)?;

    let (dpop_token, _header, _claims) = auth_dpop(dpop_key, "POST", token_endpoint)
        .context("Failed to create DPoP proof for refresh")?;

    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("grant_type", "refresh_token"),
        ("refresh_token", creds.refresh_token.as_str()),
        (
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
        ),
        ("client_assertion", client_assertion.as_str()),
    ];

    let resp = http_client
        .post(token_endpoint)
        .header("DPoP", &dpop_token)
        .form(&params)
        .send()
        .await
        .context("Token refresh request failed")?;

    // Handle DPoP nonce challenge on token endpoint
    if (resp.status() == reqwest::StatusCode::BAD_REQUEST
        || resp.status() == reqwest::StatusCode::UNAUTHORIZED)
        && let Some(nonce) = resp
            .headers()
            .get("DPoP-Nonce")
            .and_then(|v| v.to_str().ok())
    {
        let nonce_params = RefreshNonceParams {
            http_client: &http_client,
            dpop_key,
            token_endpoint,
            form_params: &params,
            nonce,
        };
        return refresh_with_nonce(&nonce_params, pool, user_id, creds).await;
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Token refresh returned {}: {}", status, body));
    }

    parse_and_store_refresh(resp, pool, user_id, creds).await
}

struct RefreshNonceParams<'a> {
    http_client: &'a reqwest::Client,
    dpop_key: &'a KeyData,
    token_endpoint: &'a str,
    form_params: &'a [(&'a str, &'a str)],
    nonce: &'a str,
}

async fn refresh_with_nonce(
    p: &RefreshNonceParams<'_>,
    pool: &SqlitePool,
    user_id: &str,
    creds: &users::AtprotoCredentials,
) -> Result<String> {
    let (_dpop_token, header, mut claims) = auth_dpop(p.dpop_key, "POST", p.token_endpoint)
        .context("Failed to create DPoP proof for refresh nonce retry")?;

    claims
        .private
        .insert("nonce".to_string(), p.nonce.to_string().into());
    let dpop_token_with_nonce = jwt::mint(p.dpop_key, &header, &claims)
        .map_err(|e| anyhow!("Failed to mint DPoP with nonce: {e}"))?;

    let resp = p
        .http_client
        .post(p.token_endpoint)
        .header("DPoP", &dpop_token_with_nonce)
        .form(p.form_params)
        .send()
        .await
        .context("Token refresh request failed (nonce retry)")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Token refresh returned {} (nonce retry): {}",
            status,
            body
        ));
    }

    parse_and_store_refresh(resp, pool, user_id, creds).await
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u32,
}

async fn parse_and_store_refresh(
    resp: reqwest::Response,
    pool: &SqlitePool,
    user_id: &str,
    creds: &users::AtprotoCredentials,
) -> Result<String> {
    let refresh_resp: RefreshResponse = resp
        .json()
        .await
        .context("Failed to parse token refresh response")?;

    let expires_at = (chrono::Utc::now()
        + chrono::Duration::seconds(refresh_resp.expires_in as i64))
    .to_rfc3339();

    if let Err(e) = users::store_atproto_credentials(
        pool,
        user_id,
        &refresh_resp.access_token,
        refresh_resp
            .refresh_token
            .as_deref()
            .unwrap_or(&creds.refresh_token),
        &creds.dpop_private_key,
        &creds.pds_url,
        &expires_at,
    )
    .await
    {
        warn!(error = %e, "Failed to update refreshed tokens");
    } else {
        info!(user_id = %user_id, "AT Protocol tokens refreshed");
    }

    Ok(refresh_resp.access_token)
}
