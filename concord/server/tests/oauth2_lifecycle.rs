use std::sync::Arc;

use axum::body::{Body, to_bytes};

use axum::http::{Request, StatusCode, header};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use concord_server::auth::authority::AuthService;

use concord_server::auth::config::AuthConfig;

use concord_server::auth::token::hash_irc_token;

use concord_server::db::pool::{create_pool, run_migrations};

use concord_server::engine::chat_engine::ChatEngine;

use concord_server::web::app_state::{AppState, HealthState};

use concord_server::web::atproto::AtprotoOAuth;

use concord_server::web::router::build_router;

use serde_json::Value;

use sha2::{Digest, Sha256};

use tokio_util::sync::CancellationToken;

use tower::ServiceExt;

async fn fixture() -> (axum::Router, sqlx::SqlitePool, String) {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('user','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','user','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let client_secret = "client-secret";
    sqlx::query(
        "INSERT INTO oauth2_apps(id,name,owner_id,client_secret,redirect_uris,scopes,
         client_type,secret_credential_id,client_secret_hash,credential_state)
         VALUES('client','Test app','user','',?, 'identify servers.read',
         'confidential','oauth-client:client',?,'active')",
    )
    .bind(serde_json::json!(["https://client.example/callback"]).to_string())
    .bind(hash_irc_token(client_secret).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (cookie, _) = auth.issue_web_session("user").await.unwrap();
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth.clone(),
        "replay",
        4000,
        100,
    ));
    let key = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key.path(), hex::encode([7_u8; 32])).unwrap();
    let vault = Arc::new(concord_server::secrets::SecretVault::load(key.path()).unwrap());
    engine.configure_integration_vault(vault.clone()).unwrap();
    let atproto = AtprotoOAuth::load_or_create(&pool, &vault).await.unwrap();
    let router = build_router(Arc::new(AppState {
        engine,
        db: pool.clone(),
        auth_config: AuthConfig {
            jwt_secret: "test-secret".into(),
            session_expiry_hours: 1,
            public_url: "http://localhost:3000".into(),
        },
        auth,
        atproto,
        secret_vault: vault,
        egress: Arc::new(concord_server::egress::EgressServices::internet().unwrap()),
        max_file_size: 1024,
        max_media_per_user: 1024,
        max_media_total: 4096,
        upload_admission: Arc::new(tokio::sync::Semaphore::new(1)),
        upload_idle_timeout: std::time::Duration::from_secs(1),
        upload_total_timeout: std::time::Duration::from_secs(2),
        max_message_length: 4000,
        admin_user_ids: Arc::from([]),
        health: Arc::new(HealthState::default()),
        shutdown: CancellationToken::new(),
        media_dir: std::env::temp_dir(),
    }));
    (router, pool, cookie)
}

fn form(values: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(values.iter().copied())
        .finish()
}

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

async fn insert_consent(pool: &sqlx::SqlitePool, token: &str) {
    sqlx::query(
        "INSERT INTO oauth2_consent_requests
         (id_hash,app_id,user_id,server_id,redirect_uri,scopes,state,code_challenge,expires_at)
         VALUES(?,'client','user','server','https://client.example/callback','identify',
                'state','challenge',datetime('now','+5 minutes'))",
    )
    .bind(hash(token))
    .execute(pool)
    .await
    .unwrap();
}

async fn post_consent(router: axum::Router, cookie: &str, token: &str) -> StatusCode {
    router
        .oneshot(
            Request::post("/oauth/authorize")
                .header(header::COOKIE, format!("concord_session={cookie}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form(&[
                    ("consent_token", token),
                    ("decision", "approve"),
                ])))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn authorize(router: &axum::Router, cookie: &str, verifier: &str) -> String {
    authorize_with_scope(router, cookie, verifier, "identify servers.read").await
}

async fn authorize_with_scope(
    router: &axum::Router,
    cookie: &str,
    verifier: &str,
    scope: &str,
) -> String {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let query = form(&[
        ("response_type", "code"),
        ("client_id", "client"),
        ("redirect_uri", "https://client.example/callback"),
        ("scope", scope),
        ("state", "opaque-state"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("server_id", "server"),
    ]);
    let response = router
        .clone()
        .oneshot(
            Request::get(format!("/oauth/authorize?{query}"))
                .header(header::COOKIE, format!("concord_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 100_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("Published by user"));
    assert!(html.contains("Access target: server Server"));
    assert!(html.contains(scope));
    let consent = html
        .split("name=consent_token value=\"")
        .nth(1)
        .and_then(|value| value.split('"').next())
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::post("/oauth/authorize")
                .header(header::COOKIE, format!("concord_session={cookie}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form(&[
                    ("consent_token", consent),
                    ("decision", "approve"),
                ])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_redirection(),
        "consent POST returned {}",
        response.status()
    );
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    let url = url::Url::parse(location).unwrap();
    assert_eq!(
        url.query_pairs().find(|(key, _)| key == "state").unwrap().1,
        "opaque-state"
    );
    url.query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned()
}

async fn public_token_request(router: &axum::Router, body: String) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body =
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap();
    (status, body)
}

async fn token_request(router: &axum::Router, body: String) -> (StatusCode, Value) {
    let basic = STANDARD.encode("client:client-secret");
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/oauth/token")
                .header(header::AUTHORIZATION, format!("Basic {basic}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body =
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap();
    (status, body)
}

#[path = "oauth2_lifecycle/credentials.rs"]
mod credentials;
#[path = "oauth2_lifecycle/oauth.rs"]
mod oauth;
