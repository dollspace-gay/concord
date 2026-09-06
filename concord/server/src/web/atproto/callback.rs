use super::{
    AppState, Arc, CookieJar, Deserialize, OAuthClient, PENDING_OAUTH_TTL_SECS, Query, Response,
    State, StatusCode, Utc, Uuid, error, fetch_bsky_profile, info, issue_session_cookie, jwk,
    oauth_complete_controlled, take_pending_oauth, users, warn,
};
use axum::response::IntoResponse;

#[derive(Deserialize)]
pub struct AtprotoCallbackParams {
    pub code: String,
    pub state: String,
    pub iss: Option<String>,
}

/// GET /api/auth/atproto/callback — exchange code for tokens, create/find user.
pub async fn atproto_callback(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Query(params): Query<AtprotoCallbackParams>,
) -> Response {
    if jar.get("concord_at_state").map(|cookie| cookie.value()) != Some(params.state.as_str()) {
        return (StatusCode::BAD_REQUEST, "OAuth browser state mismatch").into_response();
    }
    // Look up pending request
    let pending = match take_pending_oauth(&state.db, &state.secret_vault, &params.state).await {
        Ok(value) => value,
        Err(error) => {
            error!(error=%error,"Pending OAuth recovery failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "OAuth login state is unavailable",
            )
                .into_response();
        }
    };
    let Some(pending) = pending else {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid or expired state parameter",
        )
            .into_response();
    };
    if Utc::now() > pending.oauth_request.expires_at
        || Utc::now() - pending.created_at > chrono::Duration::seconds(PENDING_OAUTH_TTL_SECS)
    {
        return (StatusCode::BAD_REQUEST, "OAuth request expired").into_response();
    }

    // Verify issuer matches if provided
    if let Some(ref iss) = params.iss
        && *iss != pending.oauth_request.issuer
    {
        return (StatusCode::BAD_REQUEST, "Issuer mismatch").into_response();
    }

    let public_url = &state.auth_config.public_url;
    let Some(signing_key) = state.atproto.signing_key.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AT Protocol login is unavailable",
        )
            .into_response();
    };

    let oauth_client = OAuthClient {
        redirect_uri: format!("{}/api/auth/atproto/callback", public_url),
        client_id: format!("{}/api/auth/atproto/v2/client-metadata.json", public_url),
        private_signing_key_data: signing_key,
    };

    // Exchange authorization code for tokens
    let token_response = match oauth_complete_controlled(
        &state.egress.oauth,
        &oauth_client,
        &pending.dpop_key,
        &params.code,
        &pending.oauth_request,
        &pending.auth_server,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "Token exchange failed");
            return (StatusCode::BAD_GATEWAY, "Token exchange failed").into_response();
        }
    };
    if token_response.sub.as_deref() != Some(pending.resolved_did.as_str()) {
        return (StatusCode::BAD_GATEWAY, "Provider subject mismatch").into_response();
    }

    info!(
        scope = %token_response.scope,
        token_type = %token_response.token_type,
        expires_in = token_response.expires_in,
        "AT Protocol token exchange complete"
    );

    // The DID is in token_response.sub
    let did = match &token_response.sub {
        Some(sub) => sub.clone(),
        None => {
            error!("Token response missing sub (DID)");
            return (StatusCode::BAD_GATEWAY, "Identity verification failed").into_response();
        }
    };

    // Fetch public profile for display name and avatar
    let (display_name, avatar_url) = fetch_bsky_profile(&state.egress.oauth, &did).await;
    // Use Bluesky handle as username (permanent DID is the user_id)
    let username = pending.handle.clone();

    // Find or create user — DID is the user_id
    let user_id = match users::find_by_oauth(&state.db, "atproto", &did).await {
        Ok(Some((uid, _))) => {
            // Update username to current handle (handles can change, DIDs are permanent)
            if let Err(e) = users::update_username(&state.db, &uid, &username).await {
                warn!(error = %e, "Failed to update username/handle");
            }
            uid
        }
        Ok(None) => {
            let oauth_id = Uuid::new_v4().to_string();
            if let Err(e) = users::create_with_oauth(
                &state.db,
                &users::CreateOAuthUser {
                    user_id: &did,
                    username: &username,
                    email: None,
                    avatar_url: avatar_url.as_deref(),
                    oauth_id: &oauth_id,
                    provider: "atproto",
                    provider_id: &did,
                },
            )
            .await
            {
                error!(error = %e, "Failed to create user");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create user")
                    .into_response();
            }
            info!(user_id = %did, username = %username, "new user registered via Bluesky");
            did.clone()
        }
        Err(e) => {
            error!(error = %e, "Database error during OAuth");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    // Store AT Protocol credentials for PDS API access (blob uploads, etc.)
    // Serialize the DPoP private key as JWK JSON to preserve the private key material.
    let dpop_key_str = match jwk::generate(&pending.dpop_key) {
        Ok(wrapped) => serde_json::to_string(&wrapped).unwrap_or_default(),
        Err(e) => {
            warn!(error = %e, "Failed to serialize DPoP key as JWK");
            String::new()
        }
    };
    let expires_at =
        (Utc::now() + chrono::Duration::seconds(token_response.expires_in as i64)).to_rfc3339();
    if let Err(e) = users::store_atproto_credentials_encrypted(
        &state.db,
        &state.secret_vault,
        &user_id,
        &users::AtprotoCredentials {
            did: did.clone(),
            access_token: token_response.access_token.clone(),
            refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
            dpop_private_key: dpop_key_str,
            pds_url: pending.pds_url.clone(),
            authorization_issuer: pending.auth_server.issuer.clone(),
            token_endpoint: pending.auth_server.token_endpoint.clone(),
            token_expires_at: expires_at,
            credential_version: 0,
        },
    )
    .await
    {
        warn!(error = %e, "Failed to store AT Protocol credentials (non-fatal)");
    }

    // Store handle on every login (ensures bsky_handle is populated for profile sync)
    let _ = crate::db::queries::atproto::store_bsky_profile_sync(
        &crate::db::queries::atproto::StoreBskyProfileParams {
            pool: &state.db,
            user_id: &user_id,
            handle: &username,
            display_name: display_name.as_deref(),
            description: None,
            banner_url: None,
            followers_count: 0,
            follows_count: 0,
        },
    )
    .await;

    if let Err(error) =
        crate::config::ensure_configured_admin(&state.db, &user_id, &state.admin_user_ids).await
    {
        error!(%error, "failed to apply stable administrator bootstrap");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Administrator bootstrap could not be verified",
        )
            .into_response();
    }

    // Issue session cookie and redirect
    issue_session_cookie(state.as_ref(), &user_id).await
}
