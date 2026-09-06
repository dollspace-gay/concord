use super::{Deserialize, provider_get_json};

/// Resolve a verified handle through its DID document to a PDS URL.
pub(super) async fn resolve_handle_to_pds(
    transport: &crate::egress::ControlledHttpClient,
    handle: &str,
) -> Result<(String, String), String> {
    let did = resolve_handle(transport, handle).await?;
    let document = resolve_did_to_doc(transport, &did).await?;
    let pds = document
        .pds_endpoints()
        .first()
        .ok_or_else(|| "No PDS endpoint found".to_string())?
        .to_string();
    Ok((did, pds))
}

pub(super) async fn resolve_handle(
    transport: &crate::egress::ControlledHttpClient,
    handle: &str,
) -> Result<String, String> {
    if handle.is_empty()
        || handle.len() > 253
        || handle.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err("Invalid handle".into());
    }
    #[derive(Deserialize)]
    struct ResolveResponse {
        did: String,
    }
    let well_known = reqwest::Url::parse(&format!("https://{handle}/.well-known/atproto-did"))
        .map_err(|_| "Invalid handle".to_string())?;
    if let Ok(request) = transport
        .request(
            reqwest::Method::GET,
            well_known,
            crate::egress::RedirectPolicy::FollowSafeGet,
        )
        .map_err(anyhow::Error::from)
        && let Ok(response) = transport.send(request).await
        && response.status.is_success()
        && response.body.len() <= 2048
        && let Ok(did) = std::str::from_utf8(&response.body)
    {
        let did = did.trim();
        if did.starts_with("did:") {
            return Ok(did.into());
        }
    }
    let url = reqwest::Url::parse_with_params(
        "https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle",
        &[("handle", handle)],
    )
    .map_err(|_| "Invalid resolver URL".to_string())?;
    let resolved: ResolveResponse = provider_get_json(transport, url)
        .await
        .map_err(|_| "Handle resolution failed".to_string())?;
    if !resolved.did.starts_with("did:") {
        return Err("Resolver returned invalid DID".into());
    }
    Ok(resolved.did)
}

pub(super) async fn resolve_did_to_doc(
    transport: &crate::egress::ControlledHttpClient,
    did: &str,
) -> Result<atproto_identity::model::Document, String> {
    let url = if let Some(identifier) = did.strip_prefix("did:plc:") {
        if identifier.is_empty() || !identifier.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("Invalid PLC DID".into());
        }
        reqwest::Url::parse(&format!("https://plc.directory/{did}"))
            .map_err(|_| "Invalid PLC DID".to_string())?
    } else if let Some(identifier) = did.strip_prefix("did:web:") {
        if identifier.contains('%') {
            return Err("Encoded did:web is unsupported".into());
        }
        let mut parts = identifier.split(':');
        let host = parts.next().ok_or("Invalid did:web")?;
        if host.is_empty() {
            return Err("Invalid did:web".into());
        }
        let path: Vec<&str> = parts.collect();
        let raw = if path.is_empty() {
            format!("https://{host}/.well-known/did.json")
        } else {
            format!("https://{host}/{}/did.json", path.join("/"))
        };
        reqwest::Url::parse(&raw).map_err(|_| "Invalid did:web".to_string())?
    } else {
        return Err("Unsupported DID method".into());
    };
    provider_get_json(transport, url)
        .await
        .map_err(|_| "DID resolution failed".into())
}
