use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::auth::authority::AuthError;

use super::app_state::AppState;
use super::auth_middleware::{AuthUser, auth_error_response};

const CODE_MINUTES: i64 = 5;
const ACCESS_MINUTES: i64 = 15;
const REFRESH_DAYS: i64 = 30;

#[derive(Deserialize)]
pub struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: Option<String>,
    code_challenge: String,
    code_challenge_method: String,
    server_id: Option<String>,
}

#[derive(Deserialize)]
pub struct ConsentForm {
    consent_token: String,
    decision: String,
}

#[derive(Deserialize)]
pub struct TokenForm {
    grant_type: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    refresh_token: String,
    scope: String,
}

#[derive(Clone, Debug)]
pub struct OAuthAccess {
    pub user_id: String,
    pub credential_key: String,
    pub scopes: Vec<String>,
    pub grant_id: String,
    pub server_id: Option<String>,
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code })).into_response()
}

fn secret(prefix: &str) -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", hex::encode(bytes))
}

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn scopes(requested: &str, allowed: &str) -> Option<String> {
    let allowed: HashSet<_> = allowed.split_ascii_whitespace().collect();
    let mut requested: Vec<_> = requested.split_ascii_whitespace().collect();
    requested.sort_unstable();
    requested.dedup();
    (!requested.is_empty() && requested.iter().all(|scope| allowed.contains(scope)))
        .then(|| requested.join(" "))
}

fn redirect(uri: &str, pairs: &[(&str, &str)]) -> Result<Response, Box<Response>> {
    let mut url = reqwest::Url::parse(uri)
        .map_err(|_| Box::new(error(StatusCode::BAD_REQUEST, "invalid_request")))?;
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    Ok(Redirect::to(url.as_str()).into_response())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

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
    let consent = secret("cc_consent_");
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
    let code = secret("cc_code_");
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

fn client_credentials(headers: &HeaderMap, form: &TokenForm) -> Option<(String, Option<String>)> {
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

async fn validate_client(
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

async fn exchange_code(
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

async fn issue_pair(
    mut tx: sqlx::Transaction<'static, sqlx::Sqlite>,
    grant: &str,
    family: Option<&str>,
    scopes: &str,
) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    let family = family.unwrap_or(&id).to_owned();
    let access = secret("cc_oauth_access_");
    let refresh = secret("cc_oauth_refresh_");
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

async fn rotate_refresh(
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
    let access = secret("cc_oauth_access_");
    let new_refresh = secret("cc_oauth_refresh_");
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

pub async fn authenticate_access(state: &AppState, token: &str) -> Result<OAuthAccess, Response> {
    let mut connection = state
        .db
        .acquire()
        .await
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"))?;
    authenticate_access_in(&mut connection, token).await
}

async fn authenticate_access_in(
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

async fn bearer_access(state: &AppState, headers: &HeaderMap) -> Result<OAuthAccess, Response> {
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

fn clear_session_cookie(public_url: &str) -> Response {
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
