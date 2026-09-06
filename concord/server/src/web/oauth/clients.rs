use super::{AppState, HeaderMap, Response, STANDARD, StatusCode, TokenForm, error};
use base64::Engine as _;

pub(super) fn client_credentials(
    headers: &HeaderMap,
    form: &TokenForm,
) -> Option<(String, Option<String>)> {
    if let Some(encoded) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "))
    {
        if form.client_id.is_some() || form.client_secret.is_some() {
            return None;
        }
        let decoded = String::from_utf8(STANDARD.decode(encoded).ok()?).ok()?;
        let (id, password) = decoded.split_once(':')?;
        return Some((id.to_owned(), Some(password.to_owned())));
    }
    Some((form.client_id.clone()?, form.client_secret.clone()))
}

pub(super) async fn validate_client(
    state: &AppState,
    connection: &mut sqlx::SqliteConnection,
    id: &str,
    password: Option<&str>,
) -> Result<(), Response> {
    let client: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT client_type,client_secret_hash FROM oauth2_apps WHERE id=?
         AND client_type IN ('confidential','public') AND credential_state='active'
         AND revoked_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"))?;
    let Some((client_type, stored)) = client else {
        return Err(error(StatusCode::UNAUTHORIZED, "invalid_client"));
    };
    if client_type == "public" {
        return password
            .is_none()
            .then_some(())
            .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "invalid_client"));
    }
    let (Some(password), Some(stored)) = (password, stored) else {
        return Err(error(StatusCode::UNAUTHORIZED, "invalid_client"));
    };
    match state
        .auth
        .verify_secret_hash(password.to_owned(), stored)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(error(StatusCode::UNAUTHORIZED, "invalid_client")),
        Err(_) => Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        )),
    }
}
