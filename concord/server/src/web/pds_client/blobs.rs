use super::{
    BlobRef, DpopRequest, KeyData, PdsRequestError, PdsSession, PinBlobRequest, Result,
    UploadBlobResponse, anyhow, deserialize_dpop_key, do_dpop_request, pin_blob_with_record,
    refresh_access_token, users, warn,
};
use anyhow::Context;

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
pub(super) async fn do_upload(
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

pub(super) fn finalize_blob_ref(resp: &UploadBlobResponse, pds_url: &str, did: &str) -> BlobRef {
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
