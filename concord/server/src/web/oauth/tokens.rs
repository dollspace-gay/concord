use super::{
    ACCESS_MINUTES, AppState, Arc, Form, HeaderMap, Json, REFRESH_DAYS, Response, State,
    StatusCode, TokenForm, TokenResponse, client_credentials, error, hash, rotate_refresh, scopes,
    secret, validate_client,
};
use axum::response::IntoResponse;

pub async fn token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    let Some((client_id, password)) = client_credentials(&headers, &form) else {
        return error(StatusCode::UNAUTHORIZED, "invalid_client");
    };
    let (_permit, mut tx) = match state.engine.begin_admitted_write().await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if let Err(response) = validate_client(&state, &mut tx, &client_id, password.as_deref()).await {
        return response;
    }
    match form.grant_type.as_str() {
        "authorization_code" => exchange_code(tx, &client_id, &form).await,
        "refresh_token" => rotate_refresh(tx, &client_id, &form).await,
        _ => error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    }
}

pub(super) async fn exchange_code(
    mut tx: sqlx::Transaction<'static, sqlx::Sqlite>,
    client_id: &str,
    form: &TokenForm,
) -> Response {
    let (Some(code), Some(uri), Some(verifier)) = (
        form.code.as_deref(),
        form.redirect_uri.as_deref(),
        form.code_verifier.as_deref(),
    ) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if reqwest::Url::parse(uri).is_err() {
        return error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let code = match crate::db::queries::oauth2::consume_authorization_code(
        &mut tx,
        &hash(code),
        client_id,
        uri,
        verifier,
    )
    .await
    {
        Ok(Some(code)) => code,
        Ok(None) => return error(StatusCode::BAD_REQUEST, "invalid_grant"),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let current_scopes: Option<String> = match sqlx::query_scalar(
        "SELECT scopes FROM oauth2_apps WHERE id=? AND credential_state='active'
         AND revoked_at IS NULL",
    )
    .bind(client_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if current_scopes
        .as_deref()
        .and_then(|allowed| scopes(&code.scopes, allowed))
        .as_deref()
        != Some(code.scopes.as_str())
    {
        return error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let resource = code.server_id.as_deref().unwrap_or("");
    let grant: String = match sqlx::query_scalar(
        "INSERT INTO oauth2_grants(id,app_id,user_id,server_id,resource_key,scopes,state)
         VALUES(?,?,?,?,?,?,'active') ON CONFLICT(app_id,user_id,resource_key) DO UPDATE SET
         scopes=excluded.scopes,state='active',revoked_at=NULL,
         grant_version=oauth2_grants.grant_version+1 RETURNING id",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(client_id)
    .bind(&code.user_id)
    .bind(&code.server_id)
    .bind(resource)
    .bind(&code.scopes)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(grant) => grant,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if sqlx::query(
        "UPDATE oauth2_tokens SET revoked_at=COALESCE(revoked_at,datetime('now'))
         WHERE grant_id=?",
    )
    .bind(&grant)
    .execute(&mut *tx)
    .await
    .is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    issue_pair(tx, &grant, None, &code.scopes).await
}

pub(super) async fn issue_pair(
    mut tx: sqlx::Transaction<'static, sqlx::Sqlite>,
    grant: &str,
    family: Option<&str>,
    scopes: &str,
) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    let family = family.unwrap_or(&id).to_owned();
    let Ok(access) = secret("cc_oauth_access_") else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let Ok(refresh) = secret("cc_oauth_refresh_") else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let access_expiry = (chrono::Utc::now() + chrono::Duration::minutes(ACCESS_MINUTES))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let refresh_expiry = (chrono::Utc::now() + chrono::Duration::days(REFRESH_DAYS))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    if sqlx::query(
        "INSERT INTO oauth2_tokens(id,grant_id,token_family_id,access_token_hash,
         refresh_token_hash,scopes,access_expires_at,refresh_expires_at)
         VALUES(?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(grant)
    .bind(family)
    .bind(hash(&access))
    .bind(hash(&refresh))
    .bind(scopes)
    .bind(access_expiry)
    .bind(refresh_expiry)
    .execute(&mut *tx)
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    Json(TokenResponse {
        access_token: access,
        token_type: "Bearer",
        expires_in: ACCESS_MINUTES * 60,
        refresh_token: refresh,
        scope: scopes.to_owned(),
    })
    .into_response()
}
