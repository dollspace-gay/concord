use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::auth::token::{generate_irc_token, hash_irc_token};
use crate::db::queries::users;
use crate::engine::events::HistoryMessage;

use super::app_state::AppState;
use super::auth_middleware::AuthUser;

// ── Channel endpoints (public) ──────────────────────────

#[derive(Deserialize)]
pub struct HistoryParams {
    pub before: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct HistoryResponse {
    pub channel: String,
    pub messages: Vec<HistoryMessage>,
    pub has_more: bool,
}

pub async fn get_channel_history(
    State(state): State<Arc<AppState>>,
    Path(channel_name): Path<String>,
    Query(params): Query<HistoryParams>,
) -> impl IntoResponse {
    let channel = if channel_name.starts_with('#') {
        channel_name
    } else {
        format!("#{}", channel_name)
    };

    let limit = params.limit.unwrap_or(50).min(200);

    match state
        .engine
        .fetch_history(&channel, params.before.as_deref(), limit)
        .await
    {
        Ok((messages, has_more)) => Json(HistoryResponse {
            channel,
            messages,
            has_more,
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn get_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.engine.list_channels())
}

// ── Auth status (public) ────────────────────────────────

#[derive(Serialize)]
pub struct AuthStatusResponse {
    pub authenticated: bool,
    pub providers: Vec<String>,
}

/// GET /api/auth/status — returns available providers and auth state.
pub async fn auth_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut providers = Vec::new();
    if state.auth_config.github.is_some() {
        providers.push("github".to_string());
    }
    if state.auth_config.google.is_some() {
        providers.push("google".to_string());
    }

    Json(AuthStatusResponse {
        authenticated: false, // caller can check /api/me instead
        providers,
    })
}

// ── User profile (authenticated) ────────────────────────

#[derive(Serialize)]
pub struct UserProfile {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

/// GET /api/me — return the current user's profile.
pub async fn get_me(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match users::get_user(&state.db, &auth.user_id).await {
        Ok(Some((id, username, email, avatar_url))) => Json(UserProfile {
            id,
            username,
            email,
            avatar_url,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to fetch user profile");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

// ── IRC token management (authenticated) ─────────────────

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub id: String,
    pub token: String, // plaintext, shown only once
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct IrcTokenInfo {
    pub id: String,
    pub label: Option<String>,
    pub last_used: Option<String>,
    pub created_at: String,
}

/// POST /api/tokens — generate a new IRC access token.
pub async fn create_irc_token(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(body): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    let token = generate_irc_token();
    let hash = match hash_irc_token(&token) {
        Ok(h) => h,
        Err(e) => {
            error!(error = %e, "Failed to hash IRC token");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Token creation failed").into_response();
        }
    };

    let token_id = Uuid::new_v4().to_string();

    if let Err(e) =
        users::create_irc_token(&state.db, &token_id, &auth.user_id, &hash, body.label.as_deref())
            .await
    {
        error!(error = %e, "Failed to store IRC token");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Token creation failed").into_response();
    }

    Json(CreateTokenResponse {
        id: token_id,
        token, // shown only once
        label: body.label,
    })
    .into_response()
}

/// GET /api/tokens — list the current user's IRC tokens (no secrets).
pub async fn list_irc_tokens(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match users::list_irc_tokens(&state.db, &auth.user_id).await {
        Ok(rows) => {
            let tokens: Vec<IrcTokenInfo> = rows
                .into_iter()
                .map(|(id, label, last_used, created_at)| IrcTokenInfo {
                    id,
                    label,
                    last_used,
                    created_at,
                })
                .collect();
            Json(tokens).into_response()
        }
        Err(e) => {
            error!(error = %e, "Failed to list IRC tokens");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

/// DELETE /api/tokens/:id — revoke an IRC token.
pub async fn delete_irc_token(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(token_id): Path<String>,
) -> impl IntoResponse {
    match users::delete_irc_token(&state.db, &token_id, &auth.user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Token not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to delete IRC token");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}
