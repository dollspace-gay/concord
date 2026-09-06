use super::{
    ACCESS_MINUTES, Json, REFRESH_DAYS, Response, Row, StatusCode, TokenForm, TokenResponse, error,
    hash, scopes, secret,
};
use axum::response::IntoResponse;

pub(super) async fn rotate_refresh(
    mut tx: sqlx::Transaction<'static, sqlx::Sqlite>,
    client_id: &str,
    form: &TokenForm,
) -> Response {
    let Some(refresh) = form.refresh_token.as_deref() else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let row = match sqlx::query(
        "SELECT t.id,t.grant_id,t.token_family_id,t.scopes,t.refresh_expires_at,
        t.rotated_to_id,t.revoked_at,g.state,a.scopes,g.server_id,g.scopes FROM oauth2_tokens t
         JOIN oauth2_grants g ON g.id=t.grant_id
         JOIN oauth2_apps a ON a.id=g.app_id JOIN users u ON u.id=g.user_id
         WHERE t.refresh_token_hash=? AND g.app_id=? AND g.state='active'
         AND a.credential_state='active' AND a.revoked_at IS NULL AND u.disabled_at IS NULL
         AND (g.server_id IS NULL OR EXISTS(SELECT 1 FROM server_members m
             WHERE m.server_id=g.server_id AND m.user_id=g.user_id
             AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=m.server_id
                            AND b.user_id=m.user_id)))",
    )
    .bind(hash(refresh))
    .bind(client_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error(StatusCode::BAD_REQUEST, "invalid_grant"),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let old_id: String = row.get(0);
    let grant: String = row.get(1);
    let family: String = row.get(2);
    let granted_scopes: String = row.get(3);
    let app_scopes: String = row.get(8);
    let grant_scopes: String = row.get(10);
    if scopes(&granted_scopes, &app_scopes).as_deref() != Some(granted_scopes.as_str())
        || scopes(&granted_scopes, &grant_scopes).as_deref() != Some(granted_scopes.as_str())
    {
        return error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let replay =
        row.get::<Option<String>, _>(5).is_some() || row.get::<Option<String>, _>(6).is_some();
    if replay {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let family_update = sqlx::query(
            "UPDATE oauth2_tokens SET revoked_at=COALESCE(revoked_at,?),reuse_detected_at=?
             WHERE token_family_id=?",
        )
        .bind(&now)
        .bind(&now)
        .bind(&family)
        .execute(&mut *tx)
        .await;
        let grant_update = sqlx::query(
            "UPDATE oauth2_grants SET state='revoked',revoked_at=?,grant_version=grant_version+1
             WHERE id=? AND state='active'",
        )
        .bind(&now)
        .bind(&grant)
        .execute(&mut *tx)
        .await;
        if !matches!(family_update, Ok(result) if result.rows_affected() > 0)
            || !matches!(grant_update, Ok(result) if result.rows_affected() == 1)
            || tx.commit().await.is_err()
        {
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
        return error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let expired = row
        .get::<Option<String>, _>(4)
        .is_none_or(|expiry| expiry <= chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());
    if expired || row.get::<String, _>(7) != "active" {
        return error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let replacement = uuid::Uuid::new_v4().to_string();
    let Ok(access) = secret("cc_oauth_access_") else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let Ok(new_refresh) = secret("cc_oauth_refresh_") else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let access_expiry = (chrono::Utc::now() + chrono::Duration::minutes(ACCESS_MINUTES))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let refresh_expiry = (chrono::Utc::now() + chrono::Duration::days(REFRESH_DAYS))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let insert = sqlx::query(
        "INSERT INTO oauth2_tokens(id,grant_id,token_family_id,access_token_hash,
         refresh_token_hash,scopes,access_expires_at,refresh_expires_at)
         VALUES(?,?,?,?,?,?,?,?)",
    )
    .bind(&replacement)
    .bind(&grant)
    .bind(&family)
    .bind(hash(&access))
    .bind(hash(&new_refresh))
    .bind(&granted_scopes)
    .bind(access_expiry)
    .bind(refresh_expiry)
    .execute(&mut *tx)
    .await;
    let rotate = sqlx::query(
        "UPDATE oauth2_tokens SET rotated_to_id=?,revoked_at=datetime('now')
         WHERE id=? AND rotated_to_id IS NULL AND revoked_at IS NULL",
    )
    .bind(&replacement)
    .bind(&old_id)
    .execute(&mut *tx)
    .await;
    if insert.is_err() || !matches!(rotate, Ok(result) if result.rows_affected() == 1) {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    if tx.commit().await.is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    Json(TokenResponse {
        access_token: access,
        token_type: "Bearer",
        expires_in: ACCESS_MINUTES * 60,
        refresh_token: new_refresh,
        scope: granted_scopes,
    })
    .into_response()
}
