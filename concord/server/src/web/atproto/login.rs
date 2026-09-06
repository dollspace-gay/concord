use super::{
    AppState, Arc, Deserialize, IntoResponse, Json, KeyType, OAuthClient, OAuthRequest,
    OAuthRequestState, PENDING_OAUTH_TTL_SECS, PendingAtprotoAuth, Query, Redirect, Response,
    SET_COOKIE, State, StatusCode, Utc, Uuid, discover_authorization_server, error, generate_key,
    jwk, oauth_init_controlled, pkce, resolve_handle_to_pds, store_pending_oauth, to_public, warn,
};

/// GET /api/auth/atproto/client-metadata.json — serves OAuth client metadata document.
pub async fn client_metadata(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let public_url = &state.auth_config.public_url;
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);

    let Some(public_jwk) = state.atproto.public_jwk.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AT Protocol login is unavailable",
        )
            .into_response();
    };
    let public_jwk_value =
        serde_json::to_value(public_jwk).expect("failed to serialize public JWK");

    let metadata = serde_json::json!({
        "client_id": client_id,
        "application_type": "web",
        "client_name": "Concord",
        "client_uri": public_url,
        "dpop_bound_access_tokens": true,
        "grant_types": ["authorization_code", "refresh_token"],
        "redirect_uris": [format!("{}/api/auth/atproto/callback", public_url)],
        "response_types": ["code"],
        "scope": "atproto transition:generic",
        "token_endpoint_auth_method": "private_key_jwt",
        "token_endpoint_auth_signing_alg": "ES256",
        "jwks": {
            "keys": [public_jwk_value]
        }
    });

    Json(metadata).into_response()
}

#[derive(Deserialize)]
pub struct AtprotoLoginParams {
    pub handle: String,
}

/// GET /api/auth/atproto/login?handle=user.bsky.social — initiate Bluesky OAuth flow.
pub async fn atproto_login(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AtprotoLoginParams>,
) -> Response {
    let handle = params.handle.trim().to_string();
    if handle.is_empty() {
        return (StatusCode::BAD_REQUEST, "Handle is required").into_response();
    }

    let public_url = &state.auth_config.public_url;
    let Some(signing_key) = state.atproto.signing_key.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AT Protocol login is unavailable",
        )
            .into_response();
    };
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);
    let redirect_uri = format!("{}/api/auth/atproto/callback", public_url);

    // Resolve handle -> DID -> DID document -> PDS endpoint
    let (resolved_did, pds_url) = match resolve_handle_to_pds(&state.egress.oauth, &handle).await {
        Ok(url) => url,
        Err(e) => {
            warn!(handle = %handle, error = %e, "Failed to resolve handle");
            return (
                StatusCode::BAD_REQUEST,
                format!("Could not resolve handle: {}", e),
            )
                .into_response();
        }
    };

    // Discover authorization server from PDS
    let auth_server = match discover_authorization_server(&state.egress.oauth, &pds_url).await {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "Failed to fetch PDS resources");
            return (
                StatusCode::BAD_GATEWAY,
                "Failed to discover authorization server",
            )
                .into_response();
        }
    };

    // Generate security parameters
    let dpop_key = generate_key(KeyType::P256Private).expect("failed to generate DPoP key");
    let (pkce_verifier, code_challenge) = pkce::generate();
    let oauth_state = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();

    let oauth_client = OAuthClient {
        redirect_uri: redirect_uri.clone(),
        client_id: client_id.clone(),
        private_signing_key_data: signing_key.clone(),
    };

    let request_state = OAuthRequestState {
        state: oauth_state.clone(),
        nonce: nonce.clone(),
        code_challenge,
        scope: "atproto transition:generic".to_string(),
    };

    // Make Pushed Authorization Request (PAR)
    let par_response = match oauth_init_controlled(
        &state.egress.oauth,
        &oauth_client,
        &dpop_key,
        Some(handle.as_str()),
        &auth_server,
        &request_state,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "PAR request failed");
            return (
                StatusCode::BAD_GATEWAY,
                format!("Authorization request failed: {}", e),
            )
                .into_response();
        }
    };

    // Serialize keys for OAuthRequest storage
    let dpop_jwk = jwk::generate(&dpop_key).unwrap_or_else(|_| {
        panic!("failed to generate DPoP JWK");
    });
    let dpop_private_key = serde_json::to_string(&dpop_jwk).expect("failed to serialize DPoP key");

    let signing_pub = match to_public(&signing_key) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AT Protocol login key is unavailable",
            )
                .into_response();
        }
    };
    let signing_pub_jwk = match jwk::generate(&signing_pub) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AT Protocol login key is unavailable",
            )
                .into_response();
        }
    };
    let signing_public_key = match serde_json::to_string(&signing_pub_jwk) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AT Protocol login key is unavailable",
            )
                .into_response();
        }
    };

    let now = Utc::now();

    // Store pending request for callback
    let oauth_request = OAuthRequest {
        oauth_state: oauth_state.clone(),
        issuer: auth_server.issuer.clone(),
        authorization_server: auth_server.issuer.clone(),
        nonce,
        pkce_verifier,
        signing_public_key,
        dpop_private_key,
        created_at: now,
        expires_at: now + chrono::Duration::seconds(par_response.expires_in as i64),
    };

    let pending = PendingAtprotoAuth {
        oauth_request,
        dpop_key,
        handle: handle.clone(),
        auth_server: auth_server.clone(),
        pds_url: pds_url.clone(),
        resolved_did,
        created_at: Utc::now(),
    };
    if let Err(error) = store_pending_oauth(&state.db, &state.secret_vault, &pending).await {
        warn!(error=%error,"Failed to persist pending OAuth request");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth login state could not be stored",
        )
            .into_response();
    }

    // Redirect user to authorization server
    let auth_url = format!(
        "{}?client_id={}&request_uri={}",
        auth_server.authorization_endpoint,
        urlencoding::encode(&oauth_client.client_id),
        urlencoding::encode(&par_response.request_uri),
    );

    let mut response = Redirect::temporary(&auth_url).into_response();
    if let Ok(value)=format!(
        "concord_at_state={oauth_state}; HttpOnly; Secure; Path=/api/auth/atproto/callback; Max-Age={PENDING_OAUTH_TTL_SECS}; SameSite=Lax"
    ).parse() {
        response.headers_mut().append(SET_COOKIE,value);
    }
    response
}
