use super::{
    Deserialize, DpopRequest, KeyData, PdsSession, PdsXrpcParams, Result, Serialize, anyhow,
    do_dpop_request, pds_xrpc_call, users,
};
use anyhow::Context;

/// JSON body for com.atproto.repo.createRecord
#[derive(Serialize)]
pub struct CreateRecordRequest<T: Serialize> {
    pub repo: String,
    pub collection: String,
    pub record: T,
}

/// A minimal record that references a blob, pinning it in the user's PDS repo.
#[derive(Serialize)]
pub(super) struct AttachmentRecord {
    #[serde(rename = "$type")]
    pub(super) record_type: String,
    pub(super) blob: BlobObject,
    #[serde(rename = "createdAt")]
    pub(super) created_at: String,
}

/// AT Protocol blob reference object for embedding in records.
#[derive(Serialize)]
pub(super) struct BlobObject {
    #[serde(rename = "$type")]
    pub(super) blob_type: String,
    #[serde(rename = "ref")]
    pub(super) ref_link: BlobLink,
    #[serde(rename = "mimeType")]
    pub(super) mime_type: String,
    pub(super) size: usize,
}

#[derive(Serialize)]
pub(super) struct BlobLink {
    #[serde(rename = "$link")]
    pub(super) link: String,
}

pub(super) struct PinBlobRequest<'a> {
    pub(super) transport: &'a crate::egress::ControlledHttpClient,
    pub(super) dpop_key: &'a KeyData,
    pub(super) access_token: &'a str,
    pub(super) pds_url: &'a str,
    pub(super) did: &'a str,
    pub(super) cid: &'a str,
    pub(super) content_type: &'a str,
    pub(super) file_size: usize,
}

/// Create a record in the user's PDS repo that references the uploaded blob.
/// This pins the blob so it can be served via com.atproto.sync.getBlob.
pub(super) async fn pin_blob_with_record(request: &PinBlobRequest<'_>) -> Result<()> {
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
