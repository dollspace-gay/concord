use super::{AppState, Arc, AuthError, CookieJar, Redirect, Response, State, StatusCode, error};
use axum::response::IntoResponse;

pub(super) fn clear_session_cookie(public_url: &str) -> Response {
    let secure = if public_url.starts_with("https") {
        "; Secure"
    } else {
        ""
    };
    (
        [(
            axum::http::header::SET_COOKIE,
            format!("concord_session=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax{secure}"),
        )],
        Redirect::temporary("/"),
    )
        .into_response()
}

pub async fn logout(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    let Some(cookie) = jar.get("concord_session") else {
        return clear_session_cookie(&state.auth_config.public_url);
    };
    let actor = match state.auth.authenticate_web_session(cookie.value()).await {
        Ok(actor) => actor,
        Err(
            AuthError::Invalid
            | AuthError::Expired
            | AuthError::Revoked
            | AuthError::Disabled
            | AuthError::Token(_),
        ) => return clear_session_cookie(&state.auth_config.public_url),
        Err(AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_)) => {
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    match state.auth.revoke_credential(actor.credential_id()).await {
        Ok(_) => {
            // An idempotent concurrent revocation can report no changed row;
            // either way, no live transport may retain this durable credential.
            state.auth.cancel_live_credential(actor.credential_id());
            clear_session_cookie(&state.auth_config.public_url)
        }
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    }
}
