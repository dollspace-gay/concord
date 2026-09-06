use super::{
    DpopRequest, KeyData, PdsRequestError, PdsXrpcParams, Result, anyhow, jwk,
    refresh_access_token, request_dpop, users, warn,
};
use anyhow::Context;
use atproto_oauth::jwt;

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

pub(super) async fn do_dpop_request(request: &DpopRequest<'_>) -> Result<Vec<u8>, PdsRequestError> {
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

pub(super) async fn do_dpop_request_with_nonce(
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

pub(super) async fn send_dpop(
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
pub(super) fn deserialize_dpop_key(dpop_private_key_json: &str) -> Result<KeyData> {
    let wrapped_jwk: jwk::WrappedJsonWebKey = serde_json::from_str(dpop_private_key_json)
        .context("Failed to deserialize stored DPoP key from JWK")?;
    jwk::to_key_data(&wrapped_jwk).map_err(|e| anyhow!("Invalid stored DPoP JWK: {e:?}"))
}
