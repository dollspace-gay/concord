#![cfg(feature = "browser-fixtures")]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use concord_server::auth::authority::AuthService;
use concord_server::auth::config::AuthConfig;
use concord_server::db::pool::{create_pool, run_migrations};
use concord_server::egress::EgressServices;
use concord_server::engine::chat_engine::ChatEngine;
use concord_server::engine::events::ChatEvent;
use concord_server::engine::user_session::Protocol;
use concord_server::secrets::SecretVault;
use concord_server::web::app_state::{AppState, HealthState};
use concord_server::web::atproto::AtprotoOAuth;
use concord_server::web::router::build_router;
use sqlx::{FromRow, SqlitePool};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[derive(Debug, Eq, FromRow, PartialEq)]
struct ProfileSnapshot {
    avatar_url: Option<String>,
    bio: Option<String>,
    pronouns: Option<String>,
    banner_url: Option<String>,
    bsky_handle: Option<String>,
    bsky_display_name: Option<String>,
    bsky_description: Option<String>,
    bsky_banner_url: Option<String>,
    bsky_followers_count: Option<i64>,
    bsky_follows_count: Option<i64>,
    last_profile_sync: Option<String>,
}

async fn snapshot(pool: &SqlitePool) -> ProfileSnapshot {
    sqlx::query_as(
        "SELECT u.avatar_url,p.bio,p.pronouns,p.banner_url,oa.bsky_handle, \
         oa.bsky_display_name,oa.bsky_description,oa.bsky_banner_url, \
         oa.bsky_followers_count,oa.bsky_follows_count,oa.last_profile_sync \
         FROM users u JOIN user_profiles p ON p.user_id=u.id \
         JOIN oauth_accounts oa ON oa.user_id=u.id AND oa.provider='atproto' \
         WHERE u.id='local-user-42'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn held_profile_provider() -> (
    std::net::SocketAddr,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "provider request ended before its headers");
            request.extend_from_slice(&buffer[..read]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(
            request
                .starts_with("GET /xrpc/app.bsky.actor.getProfile?actor=did%3Aplc%3Aremote-alice "),
            "route must fetch the linked DID: {request}"
        );
        entered_tx.send(()).unwrap();
        release_rx.await.unwrap();

        let body = serde_json::json!({
            "did": "did:plc:remote-alice",
            "handle": "alice.example",
            "displayName": "Remote Alice",
            "description": "remote biography",
            "avatar": "https://cdn.example/remote-avatar.jpg",
            "banner": "https://cdn.example/remote-banner.jpg",
            "followersCount": 17,
            "followsCount": 9,
            "postsCount": 4
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    (address, entered_rx, release_tx, task)
}

#[tokio::test]
async fn held_profile_response_cannot_commit_after_exact_web_credential_revocation() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users(id,username,avatar_url) VALUES \
         ('local-user-42','alice','/api/uploads/10000000-0000-4000-8000-000000000001'), \
         ('viewer','viewer',NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_profiles(user_id,bio,pronouns,banner_url) VALUES \
         ('local-user-42','local biography','she/her', \
          '/api/uploads/20000000-0000-4000-8000-000000000002')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO oauth_accounts( \
             id,user_id,provider,provider_id,bsky_handle,bsky_display_name, \
             bsky_description,bsky_banner_url,bsky_followers_count,bsky_follows_count) \
         VALUES('at-account','local-user-42','atproto','did:plc:remote-alice', \
                'old.example','Old Alice','old remote biography', \
                'https://cdn.example/old-banner.jpg',3,2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','local-user-42')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('server','local-user-42','owner'),('server','viewer','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();

    let auth = AuthService::new(pool.clone(), "route-test-secret".into(), 1);
    let (cookie, sync_actor) = auth.issue_web_session("local-user-42").await.unwrap();
    let (_, viewer_actor) = auth.issue_web_session("viewer").await.unwrap();
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth.clone(),
        "route-test-replay",
        4000,
        100,
    ));
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (viewer_session, mut viewer_events) = engine
        .connect(
            Some("viewer".into()),
            "viewer".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(viewer_session, viewer_actor)
        .unwrap();
    engine
        .join_channel(viewer_session, "server", "#general")
        .await
        .unwrap();
    while viewer_events.try_recv().is_ok() {}

    let before = snapshot(&pool).await;
    let (provider_address, provider_entered, release_provider, provider_task) =
        held_profile_provider().await;
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([7_u8; 32])).unwrap();
    let vault = Arc::new(SecretVault::load(key_file.path()).unwrap());
    engine.configure_integration_vault(vault.clone()).unwrap();
    let atproto = AtprotoOAuth::load_or_create(&pool, &vault).await.unwrap();
    let router = build_router(Arc::new(AppState {
        engine: engine.clone(),
        db: pool.clone(),
        auth_config: AuthConfig {
            jwt_secret: "route-test-secret".into(),
            session_expiry_hours: 1,
            public_url: "http://localhost:3000".into(),
        },
        auth: auth.clone(),
        atproto,
        secret_vault: vault,
        egress: Arc::new(EgressServices::profile_fixture(provider_address)),
        max_file_size: 1024,
        max_media_per_user: 1024,
        max_media_total: 4096,
        upload_admission: Arc::new(tokio::sync::Semaphore::new(1)),
        upload_idle_timeout: Duration::from_secs(1),
        upload_total_timeout: Duration::from_secs(2),
        max_message_length: 4000,
        admin_user_ids: Arc::from([]),
        health: Arc::new(HealthState::default()),
        shutdown: CancellationToken::new(),
        media_dir: std::env::temp_dir(),
    }));

    let request = tokio::spawn(
        router.oneshot(
            Request::post("/api/bluesky/sync-profile")
                .header(header::COOKIE, format!("concord_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        ),
    );
    tokio::time::timeout(Duration::from_secs(2), provider_entered)
        .await
        .expect("profile route did not enter provider I/O")
        .expect("profile provider stopped before the request arrived");
    assert_ne!(sync_actor.user_id().as_str(), "did:plc:remote-alice");
    assert!(
        auth.revoke_credential(sync_actor.credential_id())
            .await
            .unwrap()
    );
    release_provider.send(()).unwrap();

    let response = request.await.unwrap().unwrap();
    provider_task.await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(snapshot(&pool).await, before);
    let profile_projection = tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            match viewer_events.recv().await {
                Some(ChatEvent::UserProfile { profile }) => break Some(profile),
                Some(_) => {}
                None => break None,
            }
        }
    })
    .await;
    assert!(
        profile_projection.is_err(),
        "revoked profile sync emitted a live profile projection"
    );
}
