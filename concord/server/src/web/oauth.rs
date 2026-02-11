use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use oauth2::reqwest::async_http_client;
use oauth2::{AuthorizationCode, CsrfToken, Scope, TokenResponse};
use serde::Deserialize;
use tracing::{error, info};
use uuid::Uuid;

use crate::auth::config::AuthConfig;
use crate::auth::oauth;
use crate::auth::token::create_session_token;
use crate::db::queries::users;

use super::app_state::AppState;

#[derive(Deserialize)]
pub struct OAuthCallback {
    pub code: String,
    pub state: Option<String>,
}

// ── GitHub ───────────────────────────────────────────────

/// GET /api/auth/github — redirect to GitHub OAuth
pub async fn github_login(State(state): State<Arc<AppState>>) -> Response {
    let Some(ref gh_config) = state.auth_config.github else {
        return (StatusCode::NOT_FOUND, "GitHub OAuth not configured").into_response();
    };

    let client = oauth::github_client(gh_config, &state.auth_config.public_url);
    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("read:user".into()))
        .add_scope(Scope::new("user:email".into()))
        .url();

    Redirect::temporary(auth_url.as_str()).into_response()
}

/// GET /api/auth/github/callback — exchange code for token, create/find user
pub async fn github_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OAuthCallback>,
) -> Response {
    let Some(ref gh_config) = state.auth_config.github else {
        return (StatusCode::NOT_FOUND, "GitHub OAuth not configured").into_response();
    };

    let client = oauth::github_client(gh_config, &state.auth_config.public_url);

    // Exchange authorization code for access token
    let token_result = client
        .exchange_code(AuthorizationCode::new(params.code))
        .request_async(async_http_client)
        .await;

    let token = match token_result {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "GitHub token exchange failed");
            return (StatusCode::BAD_REQUEST, "OAuth token exchange failed").into_response();
        }
    };

    let access_token = token.access_token().secret();

    // Fetch user profile from GitHub
    let gh_user = match oauth::fetch_github_user(access_token).await {
        Ok(u) => u,
        Err(e) => {
            error!(error = %e, "Failed to fetch GitHub user");
            return (StatusCode::BAD_GATEWAY, "Failed to fetch user profile").into_response();
        }
    };

    let provider_id = gh_user.id.to_string();

    // Find or create user
    let user_id = match users::find_by_oauth(&state.db, "github", &provider_id).await {
        Ok(Some((uid, _))) => uid,
        Ok(None) => {
            let uid = Uuid::new_v4().to_string();
            let oauth_id = Uuid::new_v4().to_string();
            if let Err(e) = users::create_with_oauth(
                &state.db,
                &users::CreateOAuthUser {
                    user_id: &uid,
                    username: &gh_user.login,
                    email: gh_user.email.as_deref(),
                    avatar_url: gh_user.avatar_url.as_deref(),
                    oauth_id: &oauth_id,
                    provider: "github",
                    provider_id: &provider_id,
                },
            )
            .await
            {
                error!(error = %e, "Failed to create user");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create user")
                    .into_response();
            }
            info!(user_id = %uid, username = %gh_user.login, "new user registered via GitHub");
            uid
        }
        Err(e) => {
            error!(error = %e, "Database error during OAuth");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    // Create JWT session
    issue_session_cookie(&state.auth_config, &user_id)
}

// ── Google ───────────────────────────────────────────────

/// GET /api/auth/google — redirect to Google OAuth
pub async fn google_login(State(state): State<Arc<AppState>>) -> Response {
    let Some(ref g_config) = state.auth_config.google else {
        return (StatusCode::NOT_FOUND, "Google OAuth not configured").into_response();
    };

    let client = oauth::google_client(g_config, &state.auth_config.public_url);
    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("openid".into()))
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .url();

    Redirect::temporary(auth_url.as_str()).into_response()
}

/// GET /api/auth/google/callback — exchange code for token, create/find user
pub async fn google_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OAuthCallback>,
) -> Response {
    let Some(ref g_config) = state.auth_config.google else {
        return (StatusCode::NOT_FOUND, "Google OAuth not configured").into_response();
    };

    let client = oauth::google_client(g_config, &state.auth_config.public_url);

    let token_result = client
        .exchange_code(AuthorizationCode::new(params.code))
        .request_async(async_http_client)
        .await;

    let token = match token_result {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "Google token exchange failed");
            return (StatusCode::BAD_REQUEST, "OAuth token exchange failed").into_response();
        }
    };

    let access_token = token.access_token().secret();

    let g_user = match oauth::fetch_google_user(access_token).await {
        Ok(u) => u,
        Err(e) => {
            error!(error = %e, "Failed to fetch Google user");
            return (StatusCode::BAD_GATEWAY, "Failed to fetch user profile").into_response();
        }
    };

    // Use "name" or email prefix as username
    let username = g_user
        .name
        .clone()
        .or_else(|| {
            g_user
                .email
                .as_ref()
                .map(|e| e.split('@').next().unwrap_or("user").to_string())
        })
        .unwrap_or_else(|| format!("user_{}", &g_user.sub[..8.min(g_user.sub.len())]));

    let user_id = match users::find_by_oauth(&state.db, "google", &g_user.sub).await {
        Ok(Some((uid, _))) => uid,
        Ok(None) => {
            let uid = Uuid::new_v4().to_string();
            let oauth_id = Uuid::new_v4().to_string();
            if let Err(e) = users::create_with_oauth(
                &state.db,
                &users::CreateOAuthUser {
                    user_id: &uid,
                    username: &username,
                    email: g_user.email.as_deref(),
                    avatar_url: g_user.picture.as_deref(),
                    oauth_id: &oauth_id,
                    provider: "google",
                    provider_id: &g_user.sub,
                },
            )
            .await
            {
                error!(error = %e, "Failed to create user");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create user")
                    .into_response();
            }
            info!(user_id = %uid, username = %username, "new user registered via Google");
            uid
        }
        Err(e) => {
            error!(error = %e, "Database error during OAuth");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    issue_session_cookie(&state.auth_config, &user_id)
}

// ── Helpers ──────────────────────────────────────────────

/// Create a JWT and set it as an HttpOnly cookie, then redirect to app root.
fn issue_session_cookie(auth_config: &AuthConfig, user_id: &str) -> Response {
    let jwt = match create_session_token(
        user_id,
        &auth_config.jwt_secret,
        auth_config.session_expiry_hours,
    ) {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "Failed to create JWT");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Session creation failed").into_response();
        }
    };

    let secure = if auth_config.public_url.starts_with("https") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "concord_session={}; HttpOnly; Path=/; Max-Age={}; SameSite=Lax{}",
        jwt,
        auth_config.session_expiry_hours * 3600,
        secure,
    );

    (
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::temporary("/"),
    )
        .into_response()
}

/// POST /api/auth/logout — clear the session cookie
pub async fn logout() -> Response {
    let cookie = "concord_session=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax";
    (
        [(axum::http::header::SET_COOKIE, cookie.to_string())],
        Redirect::temporary("/"),
    )
        .into_response()
}
