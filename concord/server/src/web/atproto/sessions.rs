use super::{AppState, Redirect, Response, StatusCode, error};
use axum::response::IntoResponse;

/// Create a JWT and set it as an HttpOnly cookie, then redirect to app root.
pub(super) async fn issue_session_cookie(state: &AppState, user_id: &str) -> Response {
    let jwt = match state.auth.issue_web_session(user_id).await {
        Ok((token, _actor)) => token,
        Err(e) => {
            error!(error = %e, "Failed to create JWT");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Session creation failed").into_response();
        }
    };

    let secure = if state.auth_config.public_url.starts_with("https") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "concord_session={}; HttpOnly; Path=/; Max-Age={}; SameSite=Lax{}",
        jwt,
        state.auth_config.session_expiry_hours * 3600,
        secure,
    );

    (
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::temporary("/"),
    )
        .into_response()
}
