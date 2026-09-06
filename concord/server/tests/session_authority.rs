use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::body::Bytes;
use axum::http::{Request, StatusCode};
use concord_server::auth::authority::{AuthError, AuthService, CredentialKind};
use concord_server::auth::config::AuthConfig;
use concord_server::auth::token::{create_session_token, hash_irc_token};
use concord_server::db::pool::{create_pool, run_migrations};
use concord_server::db::queries::bots;
use concord_server::db::queries::users::{self, CreateOAuthUser};
use concord_server::engine::chat_engine::ChatEngine;
use concord_server::engine::events::ChatEvent;
use concord_server::engine::permissions::{DEFAULT_EVERYONE, Permissions};
use concord_server::engine::user_session::Protocol;
use concord_server::irc::connection::handle_irc_connection_until;
use concord_server::web::app_state::{AppState, HealthState};
use concord_server::web::atproto::AtprotoOAuth;
use concord_server::web::router::build_router;
use futures_util::StreamExt;
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use uuid::Uuid;

struct HeldWriteStream {
    inner: tokio::io::DuplexStream,
    hold: Arc<AtomicBool>,
    entered: CancellationToken,
}

impl AsyncRead for HeldWriteStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}

impl AsyncWrite for HeldWriteStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        if self.hold.load(Ordering::Acquire) {
            self.entered.cancel();
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

async fn database() -> SqlitePool {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

async fn user(pool: &SqlitePool, id: &str, name: &str) {
    users::create_with_oauth(
        pool,
        &CreateOAuthUser {
            user_id: id,
            username: name,
            email: None,
            avatar_url: None,
            oauth_id: &format!("oauth-{id}"),
            provider: "test",
            provider_id: &format!("provider-{id}"),
        },
    )
    .await
    .unwrap();
}

async fn app(pool: SqlitePool, auth: AuthService) -> axum::Router {
    app_runtime(pool, auth).await.0
}

async fn app_runtime(
    pool: SqlitePool,
    auth: AuthService,
) -> (axum::Router, Arc<ChatEngine>, CancellationToken) {
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth.clone(),
        "session-secret",
        4000,
        100,
    ));
    let shutdown = CancellationToken::new();
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([9_u8; 32])).unwrap();
    let secret_vault =
        Arc::new(concord_server::secrets::SecretVault::load(key_file.path()).unwrap());
    engine
        .configure_integration_vault(secret_vault.clone())
        .unwrap();
    let atproto = AtprotoOAuth::load_or_create(&pool, &secret_vault)
        .await
        .unwrap();
    let router = build_router(Arc::new(AppState {
        engine: engine.clone(),
        db: pool,
        auth_config: AuthConfig {
            jwt_secret: "session-secret".into(),
            session_expiry_hours: 1,
            public_url: "http://localhost:3000".into(),
        },
        auth,
        atproto,
        secret_vault,
        egress: Arc::new(concord_server::egress::EgressServices::internet().unwrap()),
        max_file_size: 1024,
        max_message_length: 4000,
        admin_user_ids: Arc::from([]),
        health: Arc::new(HealthState::default()),
        shutdown: shutdown.clone(),
        media_dir: std::env::temp_dir(),
        max_media_per_user: 1024 * 1024,
        max_media_total: 10 * 1024 * 1024,
        upload_admission: Arc::new(tokio::sync::Semaphore::new(4)),
        upload_idle_timeout: std::time::Duration::from_secs(1),
        upload_total_timeout: std::time::Duration::from_secs(5),
    }));
    (router, engine, shutdown)
}

async fn execute_webhook(
    router: &axum::Router,
    webhook_id: &str,
    token: &str,
    key: &str,
    content: &str,
) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/webhooks/{webhook_id}/{token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"content": content, "idempotency_key": key}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn incoming_webhook_route_is_scoped_idempotent_and_revocable() {
    let pool = database().await;
    user(&pool, "owner", "owner").await;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('allowed','server','#allowed'),('wrong','server','#wrong')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind((concord_server::engine::permissions::Permissions::VIEW_CHANNELS
            | concord_server::engine::permissions::Permissions::SEND_MESSAGES
            | concord_server::engine::permissions::Permissions::READ_MESSAGE_HISTORY).bits() as i64)
        .execute(&pool).await.unwrap();

    let principal = "webhook:fixture";
    bots::create_bot_user_owned(&pool, principal, "fixture-hook", None, "owner")
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server',?,'member')")
        .bind(principal)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) VALUES('install',?,'server','owner','webhook:channel:allowed','active')")
        .bind(principal).execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let principal_id = concord_server::auth::authority::UserId::from_stored(principal).unwrap();
    let issued = auth
        .issue_bot_token(&principal_id, "Webhook", "bot webhook:channel:allowed")
        .await
        .unwrap();
    let webhook_id = Uuid::new_v4().to_string();
    concord_server::db::queries::webhooks::create_webhook(
        &pool,
        &concord_server::db::models::CreateWebhookParams {
            id: &webhook_id,
            server_id: "server",
            channel_id: "allowed",
            name: "Hook",
            avatar_url: None,
            webhook_type: "incoming",
            token: issued.credential_id.as_str(),
            url: None,
            created_by: "owner",
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE webhooks SET credential_id=?,principal_user_id=?,credential_state='active' WHERE id=?")
        .bind(issued.credential_id.as_str()).bind(principal).bind(&webhook_id)
        .execute(&pool).await.unwrap();
    let router = app(pool.clone(), auth.clone()).await;

    let (status, first) =
        execute_webhook(&router, &webhook_id, &issued.secret, "stable-key", "hello").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(first["replayed"], false);
    let (status, replay) =
        execute_webhook(&router, &webhook_id, &issued.secret, "stable-key", "hello").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(replay["replayed"], true);
    assert_eq!(first["message_id"], replay["message_id"]);

    user(&pool, "new-owner", "new-owner").await;
    sqlx::query("UPDATE servers SET owner_id='new-owner' WHERE id='server'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = execute_webhook(
        &router,
        &webhook_id,
        &issued.secret,
        "after-owner-loss",
        "still active",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = execute_webhook(
        &router,
        &webhook_id,
        &issued.secret,
        "stable-key",
        "changed",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE sender_id=?")
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);

    sqlx::query("INSERT INTO automod_rules(id,server_id,name,rule_type,config) VALUES('blocked-word','server','blocked','keyword','{\"words\":[\"forbidden\"]}')")
        .execute(&pool).await.unwrap();
    let (status, _) =
        execute_webhook(&router, &webhook_id, &issued.secret, "automod", "forbidden").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    sqlx::query("UPDATE channels SET slowmode_seconds=60 WHERE id='allowed'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) =
        execute_webhook(&router, &webhook_id, &issued.secret, "slowmode", "too soon").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    sqlx::query("UPDATE webhooks SET channel_id='wrong' WHERE id=?")
        .bind(&webhook_id)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) =
        execute_webhook(&router, &webhook_id, &issued.secret, "wrong-channel", "no").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    sqlx::query("UPDATE webhooks SET channel_id='allowed' WHERE id=?")
        .bind(&webhook_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now') WHERE id='install'")
        .execute(&pool).await.unwrap();
    let (status, _) = execute_webhook(&router, &webhook_id, &issued.secret, "revoked", "no").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stalled_multipart_upload_times_out_and_releases_its_reservation() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user-1')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','user-1','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let router = app(pool.clone(), auth).await;
    let prefix = b"--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"slow.txt\"\r\nContent-Type: text/plain\r\n\r\npartial";
    let body = futures_util::stream::once(async move {
        Ok::<Bytes, std::io::Error>(Bytes::from_static(prefix))
    })
    .chain(futures_util::stream::pending());

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/uploads?conversation_id={conversation}"))
                .header("cookie", format!("concord_session={token}"))
                .header(
                    "content-type",
                    "multipart/form-data; boundary=test-boundary",
                )
                .body(Body::from_stream(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let row: (String, i64) = sqlx::query_as(
        "SELECT media_state,reserved_bytes FROM attachments WHERE original_filename='slow.txt'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, ("failed".into(), 0));
}

#[tokio::test]
async fn authenticated_route_reports_session_database_outage_as_unavailable() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let router = app(pool.clone(), auth).await;
    pool.close().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

async fn post_upload(
    router: &axum::Router,
    token: &str,
    target: &str,
    filename: &str,
) -> StatusCode {
    let boundary = "media-policy-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: text/plain\r\n\r\nprivate bytes\r\n--{boundary}--\r\n"
    );
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/uploads?conversation_id={target}"))
                .header("cookie", format!("concord_session={token}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn get_upload(router: &axum::Router, token: &str, id: &str) -> StatusCode {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/{id}"))
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn patch_json(router: &axum::Router, token: &str, uri: &str, json: &str) -> StatusCode {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(uri)
                .header("cookie", format!("concord_session={token}"))
                .header("content-type", "application/json")
                .body(Body::from(json.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if !status.is_success() {
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        eprintln!(
            "PATCH {uri} returned {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    status
}

async fn managed_attachment(
    pool: &SqlitePool,
    id: &str,
    owner: &str,
    purpose: &str,
    server: Option<&str>,
    managed_user: Option<&str>,
    state: &str,
) {
    sqlx::query("INSERT INTO attachments(id,uploader_id,filename,original_filename,content_type,file_size,conversation_id,media_purpose,managed_server_id,managed_user_id,media_state,storage_backend,storage_key,sha256,ready_at) VALUES(?,?,?,?,'image/png',1,NULL,?,?,?,?,'local',?, '00',datetime('now'))")
        .bind(id).bind(owner).bind(id).bind(format!("{id}.png")).bind(purpose)
        .bind(server).bind(managed_user).bind(state).bind(format!("managed-{id}"))
        .execute(pool).await.unwrap();
}

#[tokio::test]
async fn managed_media_claims_revalidate_and_retire_replaced_assets() {
    let pool = database().await;
    for (id, name) in [
        ("owner", "owner"),
        ("manager", "manager"),
        ("member", "member"),
    ] {
        user(&pool, id, name).await;
    }
    sqlx::query("INSERT INTO servers(id,name,owner_id,icon_url) VALUES('server','Server','owner','/api/uploads/00000000-0000-4000-8000-000000000001')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role,avatar_url) VALUES('server','owner','owner',NULL),('server','manager','member',NULL),('server','member','member','/api/uploads/00000000-0000-4000-8000-000000000003')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('manage','server','Manager',?,0)")
        .bind(concord_server::engine::permissions::Permissions::MANAGE_SERVER.bits() as i64)
        .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO user_roles(server_id,user_id,role_id) VALUES('server','manager','manage')",
    )
    .execute(&pool)
    .await
    .unwrap();
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000001",
        "owner",
        "server_avatar",
        Some("server"),
        None,
        "attached",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000002",
        "manager",
        "server_avatar",
        Some("server"),
        None,
        "ready",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000003",
        "member",
        "server_member_avatar",
        Some("server"),
        Some("member"),
        "attached",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000004",
        "member",
        "server_member_avatar",
        Some("server"),
        Some("member"),
        "ready",
    )
    .await;
    managed_attachment(
        &pool,
        "00000000-0000-4000-8000-000000000005",
        "member",
        "server_avatar",
        Some("server"),
        None,
        "ready",
    )
    .await;

    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (manager_token, _) = auth.issue_web_session("manager").await.unwrap();
    let (member_token, _) = auth.issue_web_session("member").await.unwrap();
    let router = app(pool.clone(), auth).await;

    sqlx::query("INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('manager-ban','server','manager','owner')")
        .execute(&pool).await.unwrap();
    assert_eq!(
        patch_json(
            &router,
            &manager_token,
            "/api/servers/server/media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000002"}"#
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let untouched: String = sqlx::query_scalar(
        "SELECT media_state FROM attachments WHERE id='00000000-0000-4000-8000-000000000002'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(untouched, "ready");
    sqlx::query("DELETE FROM bans WHERE id='manager-ban'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        patch_json(
            &router,
            &manager_token,
            "/api/servers/server/media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000002"}"#
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let icon_states: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,media_state FROM attachments WHERE id IN ('00000000-0000-4000-8000-000000000001','00000000-0000-4000-8000-000000000002') ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        icon_states,
        vec![
            (
                "00000000-0000-4000-8000-000000000001".into(),
                "deleting".into()
            ),
            (
                "00000000-0000-4000-8000-000000000002".into(),
                "attached".into()
            )
        ]
    );

    assert_eq!(
        patch_json(
            &router,
            &member_token,
            "/api/servers/server/member-media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000005"}"#
        )
        .await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        patch_json(
            &router,
            &member_token,
            "/api/servers/server/member-media",
            r#"{"icon_url":"/api/uploads/00000000-0000-4000-8000-000000000004"}"#
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let member_states:Vec<(String,String)>=sqlx::query_as("SELECT id,media_state FROM attachments WHERE id IN ('00000000-0000-4000-8000-000000000003','00000000-0000-4000-8000-000000000004') ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        member_states,
        vec![
            (
                "00000000-0000-4000-8000-000000000003".into(),
                "deleting".into()
            ),
            (
                "00000000-0000-4000-8000-000000000004".into(),
                "attached".into()
            )
        ]
    );
}

#[tokio::test]
async fn media_routes_enforce_direct_and_private_thread_authorization() {
    let pool = database().await;
    for (id, name) in [("alice", "alice"), ("bob", "bob"), ("eve", "eve")] {
        user(&pool, id, name).await;
    }
    sqlx::query("INSERT INTO conversations(id,kind) VALUES('dm','direct')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO conversation_participants(conversation_id,user_id) VALUES('dm','alice'),('dm','bob')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO direct_conversation_pairs(conversation_id,lower_user_id,upper_user_id) VALUES('dm','alice','bob')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO direct_message_preferences(user_id,allow_from) VALUES('alice','everyone'),('bob','everyone')")
        .execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','alice')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','alice','owner'),('server','bob','member'),('server','eve','member')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind(concord_server::engine::permissions::DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('parent','server','#parent')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','private_thread','parent')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO thread_members(thread_id,user_id) VALUES('thread','bob')")
        .execute(&pool)
        .await
        .unwrap();
    let thread_conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='thread'")
            .fetch_one(&pool)
            .await
            .unwrap();

    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (alice_token, _) = auth.issue_web_session("alice").await.unwrap();
    let (bob_token, _) = auth.issue_web_session("bob").await.unwrap();
    let (eve_token, _) = auth.issue_web_session("eve").await.unwrap();
    let router = app(pool.clone(), auth).await;

    assert_eq!(
        post_upload(&router, &alice_token, "dm", "dm.txt").await,
        StatusCode::CREATED
    );
    assert_eq!(
        post_upload(&router, &eve_token, "dm", "denied.txt").await,
        StatusCode::NOT_FOUND
    );
    let dm_attachment: String =
        sqlx::query_scalar("SELECT id FROM attachments WHERE original_filename='dm.txt'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE attachments SET media_state='attached' WHERE id=?")
        .bind(&dm_attachment)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &dm_attachment).await,
        StatusCode::OK
    );
    assert_eq!(
        get_upload(&router, &eve_token, &dm_attachment).await,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE conversation_participants SET left_at=datetime('now') WHERE conversation_id='dm' AND user_id='bob'")
        .execute(&pool).await.unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &dm_attachment).await,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        post_upload(&router, &bob_token, &thread_conversation, "thread.txt").await,
        StatusCode::CREATED
    );
    assert_eq!(
        post_upload(&router, &eve_token, &thread_conversation, "hidden.txt").await,
        StatusCode::NOT_FOUND
    );
    let thread_attachment: String =
        sqlx::query_scalar("SELECT id FROM attachments WHERE original_filename='thread.txt'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE attachments SET media_state='attached' WHERE id=?")
        .bind(&thread_attachment)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &thread_attachment).await,
        StatusCode::OK
    );
    assert_eq!(
        get_upload(&router, &eve_token, &thread_attachment).await,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','bob','alice')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        get_upload(&router, &bob_token, &thread_attachment).await,
        StatusCode::NOT_FOUND
    );
}

async fn websocket_handshake(
    address: std::net::SocketAddr,
    token: &str,
    origin: Option<&str>,
) -> (tokio::net::TcpStream, String) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let origin = origin
        .map(|value| format!("Origin: {value}\r\n"))
        .unwrap_or_default();
    stream
        .write_all(
            format!(
                "GET /ws HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{origin}Cookie: concord_session={token}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(2), stream.readable())
            .await
            .unwrap();
        read.unwrap();
        match stream.try_read(&mut byte) {
            Ok(0) => break,
            Ok(_) => response.push(byte[0]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) => panic!("handshake read failed: {error}"),
        }
    }
    (stream, String::from_utf8(response).unwrap())
}

async fn send_ws_text(stream: &mut tokio::net::TcpStream, text: &str) {
    assert!(text.len() < 126);
    let mask = [0x13, 0x37, 0x42, 0x99];
    let mut frame = vec![0x81, 0x80 | text.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(
        text.as_bytes()
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).await.unwrap();
}

async fn wait_for_ws_close(stream: &mut tokio::net::TcpStream) {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut header = [0_u8; 2];
        loop {
            match tokio::io::AsyncReadExt::read_exact(stream, &mut header).await {
                Ok(_) if header[0] & 0x0f == 0x08 => return,
                Ok(_) => {
                    let mut length = usize::from(header[1] & 0x7f);
                    if length == 126 {
                        let mut extended = [0_u8; 2];
                        tokio::io::AsyncReadExt::read_exact(stream, &mut extended)
                            .await
                            .unwrap();
                        length = usize::from(u16::from_be_bytes(extended));
                    }
                    let mut payload = vec![0_u8; length];
                    tokio::io::AsyncReadExt::read_exact(stream, &mut payload)
                        .await
                        .unwrap();
                }
                Err(_) => return,
            }
        }
    })
    .await
    .expect("WebSocket remained open");
}

async fn wait_for_session(engine: &ChatEngine, nickname: &str, present: bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if engine.get_session_id_by_nick(nickname).is_some() == present {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("engine session state did not converge");
}

#[tokio::test]
async fn web_session_is_recorded_and_survives_service_restart() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let first = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, issued) = first.issue_web_session("user-1").await.unwrap();

    let restarted = AuthService::new(pool, "session-secret".into(), 1);
    let restored = restarted.authenticate_web_session(&token).await.unwrap();
    assert_eq!(restored.user_id(), issued.user_id());
    assert_eq!(restored.credential_id(), issued.credential_id());
}

#[tokio::test]
async fn unregistered_legacy_jwt_is_rejected() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool, "session-secret".into(), 1);
    let token = create_session_token("user-1", "session-secret", 1).unwrap();

    assert!(matches!(
        service.authenticate_web_session(&token).await,
        Err(AuthError::Invalid)
    ));
}

#[tokio::test]
async fn revocation_is_durable_and_cancels_live_lease() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = service.issue_web_session("user-1").await.unwrap();
    let lease = service.register_live(&actor).await.unwrap();

    assert!(
        service
            .revoke_credential(actor.credential_id())
            .await
            .unwrap()
    );
    tokio::time::timeout(Duration::from_secs(1), lease.cancelled())
        .await
        .unwrap();
    assert!(matches!(
        AuthService::new(pool, "session-secret".into(), 1)
            .authenticate_web_session(&token)
            .await,
        Err(AuthError::Revoked)
    ));
}

#[tokio::test]
async fn revocation_cancels_every_live_transport_for_exact_credential_only() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool, "session-secret".into(), 1);
    let (_, revoked_actor) = service.issue_web_session("user-1").await.unwrap();
    let (_, retained_actor) = service.issue_web_session("user-1").await.unwrap();
    let first_transport = service.register_live(&revoked_actor).await.unwrap();
    let second_transport = service.register_live(&revoked_actor).await.unwrap();
    let retained_transport = service.register_live(&retained_actor).await.unwrap();

    assert!(
        service
            .revoke_credential(revoked_actor.credential_id())
            .await
            .unwrap()
    );
    tokio::time::timeout(Duration::from_secs(1), first_transport.cancelled())
        .await
        .expect("first transport did not observe credential revocation");
    tokio::time::timeout(Duration::from_secs(1), second_transport.cancelled())
        .await
        .expect("second transport did not observe credential revocation");
    assert!(
        tokio::time::timeout(Duration::from_millis(25), retained_transport.cancelled())
            .await
            .is_err(),
        "revoking one credential cancelled another credential for the same user"
    );
    service.validate_actor(&retained_actor).await.unwrap();
}

#[tokio::test]
async fn already_expired_wait_completes_immediately() {
    tokio::time::timeout(
        Duration::from_millis(100),
        concord_server::auth::authority::wait_for_expiry(Some(chrono::Utc::now().timestamp() - 1)),
    )
    .await
    .expect("already-expired credential slept instead of completing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_issue_and_revoke_all_has_a_linearizable_result() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool, "session-secret".into(), 1);
    let existing = auth.issue_web_session("user-1").await.unwrap().1;
    let lease = auth.register_live(&existing).await.unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let issuer = {
        let auth = auth.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            auth.issue_web_session("user-1").await.unwrap()
        })
    };
    let revoker = {
        let auth = auth.clone();
        let barrier = barrier.clone();
        let user_id = existing.user_id().clone();
        tokio::spawn(async move {
            barrier.wait().await;
            auth.revoke_all_for_user(&user_id).await.unwrap()
        })
    };
    barrier.wait().await;
    let (token, raced_actor) = issuer.await.unwrap();
    let revoked_count = revoker.await.unwrap();
    assert!(revoked_count >= 1);
    tokio::time::timeout(Duration::from_secs(1), lease.cancelled())
        .await
        .expect("pre-existing live credential was not cancelled");

    let durable_result = auth.authenticate_web_session(&token).await;
    assert!(
        matches!(durable_result, Ok(ref actor) if actor.credential_id() == raced_actor.credential_id())
            || matches!(durable_result, Err(AuthError::Revoked)),
        "concurrent issuance must linearize entirely before or after revoke-all"
    );
}

#[tokio::test]
async fn expiry_and_disabled_account_are_checked_from_durable_state() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (expired_token, expired_actor) = service.issue_web_session("user-1").await.unwrap();
    sqlx::query(
        "UPDATE auth_credentials SET expires_at=unixepoch()-1, version=version+1 WHERE id=?",
    )
    .bind(expired_actor.credential_id().as_str())
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        service.authenticate_web_session(&expired_token).await,
        Err(AuthError::Expired)
    ));

    let (disabled_token, _) = service.issue_web_session("user-1").await.unwrap();
    sqlx::query("UPDATE users SET disabled_at=datetime('now') WHERE id='user-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        service.authenticate_web_session(&disabled_token).await,
        Err(AuthError::Disabled)
    ));
}

#[tokio::test]
async fn indexed_irc_and_bot_tokens_use_shared_authority() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    sqlx::query("UPDATE users SET is_bot=1 WHERE id='user-1'")
        .execute(&pool)
        .await
        .unwrap();
    let service = AuthService::new(pool, "session-secret".into(), 1);
    let web = service.issue_web_session("user-1").await.unwrap().1;

    let irc = service
        .issue_irc_token(web.user_id(), Some("terminal"))
        .await
        .unwrap();
    let irc_actor = service
        .authenticate_irc(&irc.secret, "carmilla")
        .await
        .unwrap();
    assert_eq!(irc_actor.kind(), CredentialKind::IrcToken);

    let bot = service
        .issue_bot_token(web.user_id(), "automation", "messages.read messages.write")
        .await
        .unwrap();
    let bot_actor = service.authenticate_bot(&bot.secret).await.unwrap();
    assert!(bot_actor.scopes().contains("messages.read"));
}

#[tokio::test]
async fn legacy_bot_hint_and_changed_irc_handle_remain_compatible() {
    let pool = database().await;
    user(&pool, "target", "old-handle").await;
    sqlx::query("UPDATE users SET is_bot=1 WHERE id='target'")
        .execute(&pool)
        .await
        .unwrap();
    let bot_secret = "bot_target.legacy-secret";
    let bot_hash = hash_irc_token(bot_secret).unwrap();
    sqlx::query(
        "INSERT INTO auth_credentials(id,user_id,kind,secret_hash,scopes,legacy_source_id) \
         VALUES ('legacy-bot-credential','target','bot_token',?,'messages.read','legacy-bot')",
    )
    .bind(&bot_hash)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO bot_tokens(id,user_id,token_hash,name,scopes,credential_id) \
         VALUES ('legacy-bot','target',?,'legacy','messages.read','legacy-bot-credential')",
    )
    .bind(&bot_hash)
    .execute(&pool)
    .await
    .unwrap();

    let irc_secret = "legacy-irc-secret";
    let irc_hash = hash_irc_token(irc_secret).unwrap();
    sqlx::query(
        "INSERT INTO auth_credentials(id,user_id,kind,secret_hash,scopes,legacy_source_id) \
         VALUES ('legacy-irc-credential','target','irc_token',?,'irc','legacy-irc')",
    )
    .bind(&irc_hash)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO irc_tokens(id,user_id,token_hash,label,credential_id) \
         VALUES ('legacy-irc','target',?,'legacy','legacy-irc-credential')",
    )
    .bind(&irc_hash)
    .execute(&pool)
    .await
    .unwrap();
    users::update_username(&pool, "target", "new-handle")
        .await
        .unwrap();

    let service = AuthService::new(pool, "session-secret".into(), 1);
    assert_eq!(
        service
            .authenticate_bot(bot_secret)
            .await
            .unwrap()
            .user_id()
            .as_str(),
        "target"
    );
    assert_eq!(
        service
            .authenticate_irc(irc_secret, "new-handle")
            .await
            .unwrap()
            .user_id()
            .as_str(),
        "target"
    );
}

#[tokio::test]
async fn actual_http_and_websocket_upgrade_reject_unregistered_credentials() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let legacy = create_session_token("user-1", "session-secret", 1).unwrap();
    let router = app(
        pool.clone(),
        AuthService::new(pool, "session-secret".into(), 1),
    )
    .await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", format!("concord_session={legacy}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "GET /ws HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nOrigin: http://localhost:3000\r\nCookie: concord_session={legacy}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .await
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response:?}");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_origin_and_cookie_mutation_are_rejected() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let router = app(pool, auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let (_, wrong_origin) =
        websocket_handshake(address, &token, Some("https://attacker.test")).await;
    assert!(wrong_origin.starts_with("HTTP/1.1 403"), "{wrong_origin:?}");
    let (_, missing_origin) = websocket_handshake(address, &token, None).await;
    assert!(
        missing_origin.starts_with("HTTP/1.1 403"),
        "{missing_origin:?}"
    );
    let (_, changed_cookie) = websocket_handshake(
        address,
        &format!("{token}mutated"),
        Some("http://localhost:3000"),
    )
    .await;
    assert!(
        changed_cookie.starts_with("HTTP/1.1 401"),
        "{changed_cookie:?}"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_websocket_logout_revocation_stops_commands_and_cleans_engine_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = auth.issue_web_session("user-1").await.unwrap();
    let (router, engine, _) = app_runtime(pool.clone(), auth.clone()).await;
    let logout_router = router.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, response) =
        websocket_handshake(address, &token, Some("http://localhost:3000")).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response:?}");
    wait_for_session(&engine, "carmilla", true).await;

    sqlx::query(
        "CREATE TRIGGER fail_logout_revocation BEFORE UPDATE OF revoked_at ON auth_credentials \
         BEGIN SELECT RAISE(FAIL, 'injected logout revocation failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let failed = logout_router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        failed
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .is_none(),
        "failed revocation cleared the browser cookie"
    );
    let revoked_at: Option<i64> =
        sqlx::query_scalar("SELECT revoked_at FROM auth_credentials WHERE id=?")
            .bind(actor.credential_id().as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        revoked_at.is_none(),
        "failed revocation changed durable state"
    );
    auth.authenticate_web_session(&token).await.unwrap();
    wait_for_session(&engine, "carmilla", true).await;

    sqlx::query("DROP TRIGGER fail_logout_revocation")
        .execute(&pool)
        .await
        .unwrap();
    let retried = logout_router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(retried.status().is_redirection());
    assert!(
        retried.headers()[axum::http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let revoked_at: Option<i64> =
        sqlx::query_scalar("SELECT revoked_at FROM auth_credentials WHERE id=?")
            .bind(actor.credential_id().as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked_at.is_some(), "successful retry was not durable");
    assert!(matches!(
        auth.authenticate_web_session(&token).await,
        Err(AuthError::Revoked)
    ));
    let already_revoked = logout_router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(already_revoked.status().is_redirection());
    assert!(
        already_revoked.headers()[axum::http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    wait_for_ws_close(&mut socket).await;
    wait_for_session(&engine, "carmilla", false).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_websocket_durable_expiry_rejects_next_command_and_cleans_engine_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = auth.issue_web_session("user-1").await.unwrap();
    let (router, engine, _) = app_runtime(pool.clone(), auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, response) =
        websocket_handshake(address, &token, Some("http://localhost:3000")).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response:?}");
    wait_for_session(&engine, "carmilla", true).await;

    sqlx::query(
        "UPDATE auth_credentials SET expires_at=unixepoch()-1, version=version+1 WHERE id=?",
    )
    .bind(actor.credential_id().as_str())
    .execute(&pool)
    .await
    .unwrap();
    send_ws_text(&mut socket, r#"{"type":"list_servers"}"#).await;
    wait_for_ws_close(&mut socket).await;
    wait_for_session(&engine, "carmilla", false).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_websocket_observes_shared_shutdown_and_cleans_engine_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let (router, engine, shutdown) = app_runtime(pool, auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, response) =
        websocket_handshake(address, &token, Some("http://localhost:3000")).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response:?}");
    wait_for_session(&engine, "carmilla", true).await;

    shutdown.cancel();
    wait_for_ws_close(&mut socket).await;
    wait_for_session(&engine, "carmilla", false).await;
    server.abort();
}

#[tokio::test]
async fn actual_http_logout_durably_revokes_and_notifies_live_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = auth.issue_web_session("user-1").await.unwrap();
    let lease = auth.register_live(&actor).await.unwrap();
    let router = app(pool.clone(), auth).await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());
    tokio::time::timeout(Duration::from_secs(1), lease.cancelled())
        .await
        .unwrap();
    assert!(matches!(
        AuthService::new(pool, "session-secret".into(), 1)
            .authenticate_web_session(&token)
            .await,
        Err(AuthError::Revoked)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_irc_connection_closes_and_releases_engine_session_on_revocation() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let web_actor = auth.issue_web_session("user-1").await.unwrap().1;
    let issued = auth
        .issue_irc_token(web_actor.user_id(), Some("test"))
        .await
        .unwrap();
    let credential_id = issued.credential_id.clone();
    let irc_actor = auth
        .authenticate_irc(&issued.secret, "carmilla")
        .await
        .unwrap();
    let canonical_nick = auth.canonical_irc_nickname(&irc_actor).await.unwrap();
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth.clone(),
        "session-secret",
        4000,
        100,
    ));
    let (server, client) = tokio::io::duplex(4096);
    let hold_writes = Arc::new(AtomicBool::new(false));
    let write_entered = CancellationToken::new();
    let server = HeldWriteStream {
        inner: server,
        hold: hold_writes.clone(),
        entered: write_entered.clone(),
    };
    let cancel = CancellationToken::new();
    let task = tokio::spawn(handle_irc_connection_until(
        server,
        "test-peer".into(),
        engine.clone(),
        pool,
        auth.clone(),
        cancel,
    ));
    let (reader, mut writer) = tokio::io::split(client);
    let mut reader = BufReader::new(reader);
    writer
        .write_all(
            format!(
                "PASS {}\r\nNICK carmilla\r\nUSER carmilla 0 * :Carmilla\r\n",
                issued.secret
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut line = String::new();
    loop {
        tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        if line.contains(&format!(" 001 {canonical_nick} ")) {
            break;
        }
        line.clear();
    }
    assert!(engine.get_session_id_by_nick(&canonical_nick).is_some());

    hold_writes.store(true, Ordering::Release);
    writer.write_all(b"CAP LS\r\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), write_entered.cancelled())
        .await
        .expect("IRC writer did not enter the held sink");
    auth.revoke_credential(&credential_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("revoked IRC connection remained live")
        .unwrap();
    assert!(engine.get_session_id_by_nick(&canonical_nick).is_none());
}

#[tokio::test]
async fn pins_require_manage_messages_and_bookmarks_redact_then_allow_private_cleanup() {
    let pool = database().await;
    user(&pool, "owner", "owner").await;
    user(&pool, "member", "member").await;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind(DEFAULT_EVERYONE.bits() as i64).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','member','member','private content')")
        .execute(&pool).await.unwrap();

    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;
    let engine = ChatEngine::new(pool.clone(), auth.clone(), "session-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (session_id, mut events) = engine
        .connect(
            Some("member".into()),
            "member".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine.bind_authenticated_actor(session_id, actor).unwrap();

    assert!(
        engine
            .pin_message(session_id, "server", "#general", "message")
            .await
            .is_err()
    );
    let pin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pinned_messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pin_count, 0, "message owner bypassed MANAGE_MESSAGES");

    engine
        .add_bookmark(session_id, "message", Some("remember"))
        .await
        .unwrap();
    assert!(matches!(
        events.recv().await,
        Some(ChatEvent::BookmarkAdd { .. })
    ));
    sqlx::query("UPDATE messages SET deleted_at=datetime('now') WHERE id='message'")
        .execute(&pool)
        .await
        .unwrap();
    engine.list_bookmarks(session_id).await.unwrap();
    let Some(ChatEvent::BookmarkList { bookmarks }) = events.recv().await else {
        panic!("bookmark list was not delivered");
    };
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].content, "[deleted]");

    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('deny-history','channel','role','everyone',?)")
        .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64).execute(&pool).await.unwrap();
    engine.remove_bookmark(session_id, "message").await.unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE user_id='member'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        remaining, 0,
        "history revocation prevented private bookmark cleanup"
    );
}

#[tokio::test]
async fn bot_credentials_are_gated_by_stable_owner_not_server_management() {
    let pool = database().await;
    user(&pool, "owner", "owner-nick").await;
    user(&pool, "other", "other-nick").await;
    bots::create_bot_user_owned(&pool, "bot-1", "owned-bot", None, "owner")
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let engine = ChatEngine::new(pool.clone(), auth, "session-secret", 4000, 100);
    let owner_session = engine
        .connect(
            Some("owner".into()),
            "owner-nick".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap()
        .0;
    let other_session = engine
        .connect(
            Some("other".into()),
            "other-nick".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap()
        .0;

    assert!(
        engine
            .create_bot_token(other_session, "bot-1", "forbidden", None)
            .await
            .unwrap_err()
            .contains("recorded bot owner")
    );
    engine
        .create_bot_token(owner_session, "bot-1", "owner-token", Some("messages.read"))
        .await
        .unwrap();
    assert_eq!(
        bots::list_bot_tokens(&pool, "bot-1").await.unwrap().len(),
        1
    );
}
