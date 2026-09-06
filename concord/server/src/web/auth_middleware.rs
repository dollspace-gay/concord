use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;

use crate::auth::authority::{Actor, AuthError};

use super::app_state::AppState;

/// Extractor that validates the session JWT from the `concord_session` cookie.
/// Use this in any handler that requires authentication.
pub struct AuthUser {
    pub user_id: String,
    pub actor: Actor,
}

pub enum RequestPrincipal {
    Web(Actor),
    Bot(Actor),
    OAuth(super::oauth::OAuthAccess),
}

impl RequestPrincipal {
    pub fn credential_key(&self) -> &str {
        match self {
            Self::Web(actor) | Self::Bot(actor) => actor.credential_id().as_str(),
            Self::OAuth(access) => &access.credential_key,
        }
    }
}

pub async fn request_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<RequestPrincipal>, Response> {
    let jar = CookieJar::from_headers(headers);
    let cookie = jar.get("concord_session");
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if cookie.is_some() && bearer.is_some() {
        return Err((StatusCode::UNAUTHORIZED, "Provide exactly one credential").into_response());
    }
    if let Some(cookie) = cookie {
        return state
            .auth
            .authenticate_web_session(cookie.value())
            .await
            .map(RequestPrincipal::Web)
            .map(Some)
            .map_err(|error| auth_error_response(error, "Invalid or expired session"));
    }
    let Some(token) = bearer else { return Ok(None) };
    if token.starts_with("cc_bot_") {
        return state
            .auth
            .authenticate_bot(token)
            .await
            .map(RequestPrincipal::Bot)
            .map(Some)
            .map_err(|error| auth_error_response(error, "Invalid bot token"));
    }
    if token.starts_with("cc_oauth_access_") {
        return super::oauth::authenticate_access(state, token)
            .await
            .map(RequestPrincipal::OAuth)
            .map(Some);
    }
    Err((StatusCode::UNAUTHORIZED, "Invalid bearer token").into_response())
}

pub(crate) fn auth_error_response(error: AuthError, invalid_message: &'static str) -> Response {
    match error {
        AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication service unavailable",
        )
            .into_response(),
        AuthError::Invalid
        | AuthError::Expired
        | AuthError::Revoked
        | AuthError::Disabled
        | AuthError::Token(_) => (StatusCode::UNAUTHORIZED, invalid_message).into_response(),
    }
}

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let principal = request_principal(state, &parts.headers).await?;
        let actor = match principal {
            Some(RequestPrincipal::Web(actor)) => actor,
            Some(RequestPrincipal::Bot(_) | RequestPrincipal::OAuth(_)) | None => {
                return Err((StatusCode::UNAUTHORIZED, "Browser session required").into_response());
            }
        };

        Ok(AuthUser {
            user_id: actor.user_id().as_str().to_owned(),
            actor,
        })
    }
}
