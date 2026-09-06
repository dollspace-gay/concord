use anyhow::{Result, anyhow};

use atproto_identity::key::KeyData;

use atproto_oauth::dpop::{auth_dpop, request_dpop};

use atproto_oauth::jwk;

use atproto_oauth::jwt::{Claims, Header, JoseClaims};

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

#[cfg(test)]
mod tests;

mod blobs;
mod records;
mod refresh;
mod requests;
pub use blobs::upload_blob_to_pds;
pub use records::CreateRecordRequest;
pub use records::CreateRecordResponse;
use records::PinBlobRequest;
pub use records::create_record;
use records::pin_blob_with_record;
pub use records::put_record;
use refresh::refresh_access_token;
use requests::deserialize_dpop_key;
use requests::do_dpop_request;
pub use requests::pds_xrpc_call;
