use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;

use super::app_state::AppState;

/// POST /api/auth/logout — clear the session cookie and revoke the JWT
pub async fn logout(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    // Revoke the JWT if present so it can't be reused
    if let Some(cookie) = jar.get("concord_session")
        && let Ok(claims) = crate::auth::token::validate_session_token(
            cookie.value(),
            &state.auth_config.jwt_secret,
        )
    {
        state.jwt_blocklist.revoke(&claims.jti, claims.exp);
    }

    let secure = if state.auth_config.public_url.starts_with("https") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "concord_session=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax{}",
        secure,
    );
    (
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::temporary("/"),
    )
        .into_response()
}
