use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;

use crate::auth::token::validate_session_token;

use super::app_state::AppState;

/// Extractor that validates the session JWT from the `concord_session` cookie.
/// Use this in any handler that requires authentication.
pub struct AuthUser {
    pub user_id: String,
}

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .unwrap(); // CookieJar extraction is infallible

        let cookie = jar.get("concord_session").ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, "Not authenticated").into_response()
        })?;

        let claims =
            validate_session_token(cookie.value(), &state.auth_config.jwt_secret).map_err(
                |_| (StatusCode::UNAUTHORIZED, "Invalid or expired session").into_response(),
            )?;

        Ok(AuthUser {
            user_id: claims.sub,
        })
    }
}
