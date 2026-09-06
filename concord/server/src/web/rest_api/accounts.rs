use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, Serialize, State, StatusCode,
    error,
};

#[derive(Serialize)]
pub struct AuthStatusResponse {
    pub authenticated: bool,
    pub providers: Vec<String>,
}

/// GET /api/auth/status — returns available providers and auth state.
pub async fn auth_status() -> impl IntoResponse {
    Json(AuthStatusResponse {
        authenticated: false, // caller can check /api/me instead
        providers: vec!["atproto".to_string()],
    })
}

#[derive(Serialize)]
pub struct UserProfile {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

/// GET /api/me — return the current user's profile.
pub async fn get_me(State(state): State<Arc<AppState>>, auth: AuthUser) -> impl IntoResponse {
    match state.engine.current_account_profile(&auth.actor).await {
        Ok(Some(profile)) => Json(UserProfile {
            id: profile.id,
            username: profile.username,
            email: profile.email,
            avatar_url: profile.avatar_url,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to fetch user profile");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

#[derive(Serialize)]
pub struct PublicUserProfile {
    pub username: String,
    pub avatar_url: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<String>,
}

/// GET /api/users/:nickname — look up a user's public profile by nickname.
pub async fn get_user_profile(
    State(state): State<Arc<AppState>>,
    Path(nickname): Path<String>,
) -> impl IntoResponse {
    match state.engine.public_account_profile(&nickname).await {
        Ok(Some(profile)) => Json(PublicUserProfile {
            username: profile.username,
            avatar_url: profile.avatar_url,
            provider: profile.provider,
            provider_id: profile.provider_id,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to fetch user profile");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

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
    // Enforce per-user token limit (max 25 active tokens)
    const MAX_TOKENS_PER_USER: usize = 25;
    match state.engine.list_irc_tokens_for_actor(&auth.actor).await {
        Ok(existing) if existing.len() >= MAX_TOKENS_PER_USER => {
            return (
                StatusCode::BAD_REQUEST,
                "Maximum number of IRC tokens reached (25). Delete unused tokens first.",
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, "Failed to check existing IRC tokens");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
        _ => {}
    }

    let issued = match state
        .auth
        .issue_irc_token(auth.actor.user_id(), body.label.as_deref())
        .await
    {
        Ok(issued) => issued,
        Err(e) => {
            error!(error = %e, "Failed to issue IRC credential");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Token creation failed").into_response();
        }
    };

    Json(CreateTokenResponse {
        id: issued.token_id,
        token: issued.secret, // shown only once
        label: body.label,
    })
    .into_response()
}

/// GET /api/tokens — list the current user's IRC tokens (no secrets).
pub async fn list_irc_tokens(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> impl IntoResponse {
    match state.engine.list_irc_tokens_for_actor(&auth.actor).await {
        Ok(rows) => {
            let tokens: Vec<IrcTokenInfo> = rows
                .into_iter()
                .map(|token| IrcTokenInfo {
                    id: token.id,
                    label: token.label,
                    last_used: token.last_used,
                    created_at: token.created_at,
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
    match state
        .auth
        .revoke_irc_token(&token_id, auth.actor.user_id())
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Token not found").into_response(),
        Err(e) => {
            error!(error = %e, "Failed to delete IRC token");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}
