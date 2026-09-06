use std::collections::HashSet;

use std::sync::Arc;

use axum::Json;

use axum::extract::{Form, Path, Query, State};

use axum::http::{HeaderMap, StatusCode};

use axum::response::{Html, IntoResponse, Redirect, Response};

use axum_extra::extract::CookieJar;

use base64::engine::general_purpose::STANDARD;

use rand::{TryRng, rngs::SysRng};

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

fn secret(prefix: &str) -> Result<String, rand::rngs::SysError> {
    let mut bytes = [0_u8; 32];
    SysRng.try_fill_bytes(&mut bytes)?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
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

mod access;
mod clients;
mod consent;
mod refresh;
mod sessions;
mod tokens;
pub use access::authenticate_access;
pub use access::delegated_servers;
pub use access::revoke_grant;
pub use access::userinfo;
use clients::client_credentials;
use clients::validate_client;
pub use consent::authorize_get;
pub use consent::authorize_post;
use refresh::rotate_refresh;
pub use sessions::logout;
pub use tokens::token;
