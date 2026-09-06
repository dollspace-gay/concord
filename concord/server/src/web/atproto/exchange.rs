use super::{
    AuthorizationServer, Claims, Header, JoseClaims, KeyData, OAuthClient, OAuthRequest,
    OAuthRequestState, ParResponse, StatusCode, TokenResponse, Utc, Uuid, auth_dpop,
};
use atproto_oauth::jwt;

pub(super) fn client_assertion(client: &OAuthClient, audience: &str) -> anyhow::Result<String> {
    let header: Header = client
        .private_signing_key_data
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("client assertion header failed"))?;
    let claims = Claims::new(JoseClaims {
        issuer: Some(client.client_id.clone()),
        subject: Some(client.client_id.clone()),
        audience: Some(audience.into()),
        json_web_token_id: Some(Uuid::new_v4().to_string()),
        issued_at: Some(Utc::now().timestamp() as u64),
        ..Default::default()
    });
    jwt::mint(&client.private_signing_key_data, &header, &claims)
        .map_err(|_| anyhow::anyhow!("client assertion failed"))
}

pub(super) async fn oauth_form_post(
    transport: &crate::egress::ControlledHttpClient,
    dpop_key: &KeyData,
    url: &str,
    form: &str,
) -> anyhow::Result<crate::egress::EgressResponse> {
    let url = reqwest::Url::parse(url)?;
    let (proof, _, _) = auth_dpop(dpop_key, "POST", url.as_str())
        .map_err(|_| anyhow::anyhow!("DPoP proof failed"))?;
    let mut response = oauth_form_post_once(transport, &url, &proof, form).await?;
    if matches!(
        response.status,
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
    ) && let Some(nonce) = response
        .headers
        .get("DPoP-Nonce")
        .and_then(|v| v.to_str().ok())
    {
        let (_, header, mut claims) = auth_dpop(dpop_key, "POST", url.as_str())
            .map_err(|_| anyhow::anyhow!("DPoP nonce proof failed"))?;
        claims
            .private
            .insert("nonce".into(), nonce.to_string().into());
        let proof = jwt::mint(dpop_key, &header, &claims)
            .map_err(|_| anyhow::anyhow!("DPoP nonce proof failed"))?;
        response = oauth_form_post_once(transport, &url, &proof, form).await?;
    }
    Ok(response)
}

pub(super) async fn oauth_form_post_once(
    transport: &crate::egress::ControlledHttpClient,
    url: &reqwest::Url,
    proof: &str,
    form: &str,
) -> anyhow::Result<crate::egress::EgressResponse> {
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
            reqwest::header::HeaderValue::from_str(proof)?,
        )
        .body(form.as_bytes().to_vec())
        .credentials_for(url)?;
    Ok(transport.send(request).await?)
}

pub(super) async fn oauth_init_controlled(
    transport: &crate::egress::ControlledHttpClient,
    client: &OAuthClient,
    dpop_key: &KeyData,
    login_hint: Option<&str>,
    server: &AuthorizationServer,
    state: &OAuthRequestState,
) -> anyhow::Result<ParResponse> {
    let assertion = client_assertion(client, &server.issuer)?;
    let form = {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer
            .append_pair("response_type", "code")
            .append_pair("code_challenge", &state.code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("client_id", &client.client_id)
            .append_pair("state", &state.state)
            .append_pair("redirect_uri", &client.redirect_uri)
            .append_pair("scope", &state.scope)
            .append_pair(
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            )
            .append_pair("client_assertion", &assertion);
        if let Some(hint) = login_hint {
            serializer.append_pair("login_hint", hint);
        }
        serializer.finish()
    };
    let response = oauth_form_post(
        transport,
        dpop_key,
        &server.pushed_authorization_request_endpoint,
        &form,
    )
    .await?;
    if !response.status.is_success() {
        anyhow::bail!("PAR returned status {}", response.status)
    }
    Ok(crate::egress::parse_provider_json(&response.body)?)
}

pub(super) async fn oauth_complete_controlled(
    transport: &crate::egress::ControlledHttpClient,
    client: &OAuthClient,
    dpop_key: &KeyData,
    code: &str,
    request: &OAuthRequest,
    server: &AuthorizationServer,
) -> anyhow::Result<TokenResponse> {
    let assertion = client_assertion(client, &server.issuer)?;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &client.redirect_uri)
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("code_verifier", &request.pkce_verifier)
        .append_pair(
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
        )
        .append_pair("client_assertion", &assertion)
        .finish();
    let response = oauth_form_post(transport, dpop_key, &server.token_endpoint, &form).await?;
    if !response.status.is_success() {
        anyhow::bail!("token endpoint returned status {}", response.status)
    }
    Ok(crate::egress::parse_provider_json(&response.body)?)
}
