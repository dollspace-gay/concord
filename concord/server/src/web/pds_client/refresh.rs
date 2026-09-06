use super::{
    Claims, Deserialize, Header, JoseClaims, KeyData, PdsSession, Result, account_refresh_lock,
    anyhow, auth_dpop, info, users,
};
use anyhow::Context;
use atproto_oauth::jwt;

/// Build a private_key_jwt client assertion for the given token endpoint.
pub(super) fn build_client_assertion(
    signing_key: &KeyData,
    client_id: &str,
    issuer: &str,
) -> Result<String> {
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
pub(super) struct RefreshResponse {
    pub(super) access_token: String,
    pub(super) refresh_token: Option<String>,
    pub(super) expires_in: u32,
}

pub(super) async fn refresh_access_token(
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

pub(super) async fn send_token_request(
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
