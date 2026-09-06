use super::{
    AppState, Arc, AuthUser, HeaderMap, Json, OAuthAccess, Path, Response, Row, State, StatusCode,
    auth_error_response, error, hash, scopes,
};
use axum::response::IntoResponse;

pub async fn authenticate_access(state: &AppState, token: &str) -> Result<OAuthAccess, Response> {
    let mut connection = state
        .db
        .acquire()
        .await
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"))?;
    authenticate_access_in(&mut connection, token).await
}

pub(super) async fn authenticate_access_in(
    connection: &mut sqlx::SqliteConnection,
    token: &str,
) -> Result<OAuthAccess, Response> {
    let row = sqlx::query(
        "SELECT g.user_id,t.id,t.scopes,g.id,g.server_id,a.scopes,g.scopes FROM oauth2_tokens t
         JOIN oauth2_grants g ON g.id=t.grant_id JOIN oauth2_apps a ON a.id=g.app_id
         JOIN users u ON u.id=g.user_id WHERE t.access_token_hash=?
         AND t.revoked_at IS NULL AND t.access_expires_at>datetime('now')
         AND g.state='active' AND a.credential_state='active' AND a.revoked_at IS NULL
         AND u.disabled_at IS NULL
         AND (g.server_id IS NULL OR EXISTS(SELECT 1 FROM server_members m
             WHERE m.server_id=g.server_id AND m.user_id=g.user_id
             AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=m.server_id
                            AND b.user_id=m.user_id)))",
    )
    .bind(hash(token))
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"))?
    .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "invalid_token"))?;
    let token_scopes: String = row.get(2);
    let app_scopes: String = row.get(5);
    let grant_scopes: String = row.get(6);
    if scopes(&token_scopes, &app_scopes).as_deref() != Some(token_scopes.as_str())
        || scopes(&token_scopes, &grant_scopes).as_deref() != Some(token_scopes.as_str())
    {
        return Err(error(StatusCode::UNAUTHORIZED, "invalid_token"));
    }
    Ok(OAuthAccess {
        user_id: row.get(0),
        credential_key: format!("oauth:{}", row.get::<String, _>(1)),
        scopes: token_scopes
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect(),
        grant_id: row.get(3),
        server_id: row.get(4),
    })
}

pub(super) async fn bearer_access(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<OAuthAccess, Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "invalid_token"))?;
    authenticate_access(state, token).await
}

pub async fn userinfo(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match bearer_access(&state, &headers).await {
        Ok(access) if access.scopes.iter().any(|scope| scope == "identify") => {
            Json(serde_json::json!({ "id": access.user_id, "scopes": access.scopes }))
                .into_response()
        }
        Ok(_) => error(StatusCode::FORBIDDEN, "insufficient_scope"),
        Err(response) => response,
    }
}

pub async fn delegated_servers(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let token = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        Some(token) => token,
        None => return error(StatusCode::UNAUTHORIZED, "invalid_token"),
    };
    let mut transaction = match state.db.begin().await {
        Ok(transaction) => transaction,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let access = match authenticate_access_in(&mut transaction, token).await {
        Ok(access) if access.scopes.iter().any(|scope| scope == "servers.read") => access,
        Ok(_) => return error(StatusCode::FORBIDDEN, "insufficient_scope"),
        Err(response) => return response,
    };
    let rows = sqlx::query(
        "SELECT s.id,s.name FROM servers s JOIN server_members m ON m.server_id=s.id
         WHERE m.user_id=? AND (? IS NULL OR s.id=?)
         AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id=m.user_id)
         ORDER BY s.name,s.id",
    )
    .bind(&access.user_id)
    .bind(&access.server_id)
    .bind(&access.server_id)
    .fetch_all(&mut *transaction)
    .await;
    match rows {
        Ok(rows) => {
            if transaction.commit().await.is_err() {
                return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
            }
            Json(rows.into_iter()
                .map(|row| serde_json::json!({ "id": row.get::<String, _>(0), "name": row.get::<String, _>(1) }))
                .collect::<Vec<_>>()).into_response()
        }
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    }
}

pub async fn revoke_grant(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(grant_id): Path<String>,
) -> Response {
    let (_permit, mut tx) = match state.engine.begin_admitted_write().await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if let Err(auth_error) = state.auth.validate_actor_in(&mut tx, &user.actor).await {
        return auth_error_response(auth_error, "Invalid or expired session");
    }
    let update = sqlx::query(
        "UPDATE oauth2_grants SET state='revoked',revoked_at=datetime('now'),
         grant_version=grant_version+1 WHERE id=? AND user_id=? AND state='active'",
    )
    .bind(&grant_id)
    .bind(&user.user_id)
    .execute(&mut *tx)
    .await;
    if !matches!(update, Ok(result) if result.rows_affected() == 1) {
        return error(StatusCode::NOT_FOUND, "invalid_grant");
    }
    if sqlx::query(
        "UPDATE oauth2_tokens SET revoked_at=COALESCE(revoked_at,datetime('now')) WHERE grant_id=?",
    )
    .bind(&grant_id)
    .execute(&mut *tx)
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    StatusCode::NO_CONTENT.into_response()
}
