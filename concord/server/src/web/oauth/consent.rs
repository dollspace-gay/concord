use super::{
    AppState, Arc, AuthUser, AuthorizeQuery, CODE_MINUTES, ConsentForm, Form, Html, Query,
    Response, Row, State, StatusCode, auth_error_response, error, escape, hash, redirect, scopes,
    secret,
};
use axum::response::IntoResponse;

pub async fn authorize_get(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<AuthorizeQuery>,
) -> Response {
    if query.response_type != "code"
        || query.code_challenge_method != "S256"
        || query.code_challenge.len() != 43
        || !query
            .code_challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || query.state.as_ref().is_some_and(|value| value.len() > 1024)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let (_permit, mut tx) = match state.engine.begin_admitted_write().await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if let Err(auth_error) = state.auth.validate_actor_in(&mut tx, &user.actor).await {
        return auth_error_response(auth_error, "Invalid or expired session");
    }
    let app = match sqlx::query(
        "SELECT a.name,a.redirect_uris,a.scopes,u.username,s.name FROM oauth2_apps a
         JOIN users u ON u.id=a.owner_id LEFT JOIN servers s ON s.id=? WHERE a.id=?
         AND client_type IN ('confidential','public') AND credential_state='active'
         AND revoked_at IS NULL",
    )
    .bind(&query.server_id)
    .bind(&query.client_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error(StatusCode::BAD_REQUEST, "invalid_client"),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let registered: Vec<String> = match serde_json::from_str(app.get(1)) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_client"),
    };
    if reqwest::Url::parse(&query.redirect_uri).is_err()
        || !registered.contains(&query.redirect_uri)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let redirect_uri = query.redirect_uri;
    let requested_scopes = match scopes(&query.scope, app.get(2)) {
        Some(value) => value,
        None => {
            return redirect(
                &redirect_uri,
                &[
                    ("error", "invalid_scope"),
                    ("state", query.state.as_deref().unwrap_or("")),
                ],
            )
            .unwrap_or_else(|response| *response);
        }
    };
    if let Some(server_id) = &query.server_id {
        let member = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
        )
        .bind(server_id)
        .bind(&user.user_id)
        .fetch_one(&mut *tx)
        .await;
        if !matches!(member, Ok(true)) {
            return error(StatusCode::BAD_REQUEST, "invalid_target");
        }
    }
    let Ok(consent) = secret("cc_consent_") else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let expires = (chrono::Utc::now() + chrono::Duration::minutes(CODE_MINUTES))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    if sqlx::query(
        "INSERT INTO oauth2_consent_requests
         (id_hash,app_id,user_id,server_id,redirect_uri,scopes,state,code_challenge,expires_at)
         VALUES(?,?,?,?,?,?,?,?,?)",
    )
    .bind(hash(&consent))
    .bind(&query.client_id)
    .bind(&user.user_id)
    .bind(&query.server_id)
    .bind(&redirect_uri)
    .bind(&requested_scopes)
    .bind(&query.state)
    .bind(&query.code_challenge)
    .bind(expires)
    .execute(&mut *tx)
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    let name = escape(app.get(0));
    let publisher = escape(app.get(3));
    let target = app.get::<Option<String>, _>(4).map_or_else(
        || "your account".to_owned(),
        |name| format!("server {}", escape(&name)),
    );
    Html(format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Authorize {name}</title></head>\
         <body><main><h1>Authorize {name}</h1><p>Published by {publisher}</p>\
         <p>Access target: {target}</p><p>This app requests: {}</p>\
         <form method=post action=/oauth/authorize><input type=hidden name=consent_token value=\"{}\">\
         <button name=decision value=approve>Authorize</button>\
         <button name=decision value=deny>Deny</button></form></main></body></html>",
        escape(&requested_scopes),
        escape(&consent)
    ))
    .into_response()
}

pub async fn authorize_post(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Form(form): Form<ConsentForm>,
) -> Response {
    let (_permit, mut tx) = match state.engine.begin_admitted_write().await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if let Err(auth_error) = state.auth.validate_actor_in(&mut tx, &user.actor).await {
        return auth_error_response(auth_error, "Invalid or expired session");
    }
    let request = match sqlx::query(
        "UPDATE oauth2_consent_requests SET consumed_at=datetime('now')
         WHERE id_hash=? AND user_id=? AND consumed_at IS NULL AND expires_at>datetime('now')
         AND EXISTS(SELECT 1 FROM oauth2_apps a WHERE a.id=oauth2_consent_requests.app_id
                    AND a.credential_state='active' AND a.revoked_at IS NULL)
         AND (server_id IS NULL OR EXISTS(
             SELECT 1 FROM server_members m
             WHERE m.server_id=oauth2_consent_requests.server_id AND m.user_id=?
             AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=m.server_id
                            AND b.user_id=m.user_id)))
         RETURNING app_id,server_id,redirect_uri,scopes,state,code_challenge",
    )
    .bind(hash(&form.consent_token))
    .bind(&user.user_id)
    .bind(&user.user_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let uri: String = request.get(2);
    let state_value: Option<String> = request.get(4);
    let requested_scopes: String = request.get(3);
    let current_scopes: Option<String> = match sqlx::query_scalar(
        "SELECT scopes FROM oauth2_apps WHERE id=? AND credential_state='active'
         AND revoked_at IS NULL",
    )
    .bind(request.get::<String, _>(0))
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    if current_scopes
        .as_deref()
        .and_then(|allowed| scopes(&requested_scopes, allowed))
        .as_deref()
        != Some(requested_scopes.as_str())
    {
        return error(StatusCode::BAD_REQUEST, "invalid_scope");
    }
    if form.decision != "approve" {
        if tx.commit().await.is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
        return redirect(
            &uri,
            &[
                ("error", "access_denied"),
                ("state", state_value.as_deref().unwrap_or("")),
            ],
        )
        .unwrap_or_else(|response| *response);
    }
    let Ok(code) = secret("cc_code_") else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let expires = (chrono::Utc::now() + chrono::Duration::minutes(CODE_MINUTES))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    if sqlx::query(
        "INSERT INTO oauth2_codes
         (id,code_hash,app_id,user_id,server_id,redirect_uri,scopes,code_challenge,
          code_challenge_method,expires_at) VALUES(?,?,?,?,?,?,?,?, 'S256',?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(hash(&code))
    .bind(request.get::<String, _>(0))
    .bind(&user.user_id)
    .bind(request.get::<Option<String>, _>(1))
    .bind(&uri)
    .bind(&requested_scopes)
    .bind(request.get::<String, _>(5))
    .bind(expires)
    .execute(&mut *tx)
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    redirect(
        &uri,
        &[
            ("code", &code),
            ("state", state_value.as_deref().unwrap_or("")),
        ],
    )
    .unwrap_or_else(|response| *response)
}
