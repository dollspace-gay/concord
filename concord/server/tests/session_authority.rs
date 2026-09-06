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

#[path = "session_authority/administration.rs"]
mod administration;
#[path = "session_authority/credentials.rs"]
mod credentials;
#[path = "session_authority/media.rs"]
mod media;
#[path = "session_authority/transports.rs"]
mod transports;
