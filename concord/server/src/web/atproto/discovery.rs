use super::{AuthorizationServer, Deserialize};

pub(super) async fn provider_get_json<T: serde::de::DeserializeOwned>(
    transport: &crate::egress::ControlledHttpClient,
    url: reqwest::Url,
) -> anyhow::Result<T> {
    let request = transport
        .request(
            reqwest::Method::GET,
            url,
            crate::egress::RedirectPolicy::FollowSafeGet,
        )?
        .header(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    let response = transport.send(request).await?;
    if !response.status.is_success() {
        anyhow::bail!("provider returned status {}", response.status)
    }
    Ok(crate::egress::parse_provider_json(&response.body)?)
}

#[derive(Deserialize)]
pub(super) struct ResourceMetadata {
    pub(super) resource: String,
    pub(super) authorization_servers: Vec<String>,
}

pub(super) async fn discover_authorization_server(
    transport: &crate::egress::ControlledHttpClient,
    pds: &str,
) -> anyhow::Result<AuthorizationServer> {
    let pds_url = reqwest::Url::parse(pds)?;
    let resource_url = pds_url.join("/.well-known/oauth-protected-resource")?;
    let resource: ResourceMetadata = provider_get_json(transport, resource_url).await?;
    if resource.resource.trim_end_matches('/') != pds_url.as_str().trim_end_matches('/')
        || resource.authorization_servers.len() != 1
    {
        anyhow::bail!("invalid protected resource metadata")
    }
    let issuer = reqwest::Url::parse(&resource.authorization_servers[0])?;
    let metadata_url = issuer.join("/.well-known/oauth-authorization-server")?;
    let metadata: AuthorizationServer = provider_get_json(transport, metadata_url).await?;
    if metadata.issuer.trim_end_matches('/') != issuer.as_str().trim_end_matches('/')
        || !metadata.authorization_response_iss_parameter_supported
        || !metadata.client_id_metadata_document_supported
        || !metadata
            .code_challenge_methods_supported
            .iter()
            .any(|v| v == "S256")
        || !metadata
            .dpop_signing_alg_values_supported
            .iter()
            .any(|v| v == "ES256")
        || !metadata
            .grant_types_supported
            .iter()
            .any(|v| v == "authorization_code")
        || !metadata
            .grant_types_supported
            .iter()
            .any(|v| v == "refresh_token")
        || !metadata.require_pushed_authorization_requests
        || !metadata
            .response_types_supported
            .iter()
            .any(|v| v == "code")
        || !metadata
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|v| v == "private_key_jwt")
        || !metadata
            .token_endpoint_auth_signing_alg_values_supported
            .iter()
            .any(|v| v == "ES256")
    {
        anyhow::bail!("authorization server metadata failed validation")
    }
    for endpoint in [
        &metadata.authorization_endpoint,
        &metadata.pushed_authorization_request_endpoint,
        &metadata.token_endpoint,
    ] {
        let url = reqwest::Url::parse(endpoint)?;
        if url.origin() != issuer.origin() {
            anyhow::bail!("authorization endpoint origin mismatch")
        }
    }
    Ok(metadata)
}
