use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tempfile::tempdir;
use tower::ServiceExt as _;

use super::{
    append_database_metrics, append_runtime_metrics, collect_metrics, probe_media_storage,
    spawn_dependency_probe, spawn_metrics_work,
};

async fn application() -> (
    axum::Router,
    Arc<super::super::app_state::AppState>,
    tempfile::TempDir,
) {
    let directory = tempdir().unwrap();
    let database = directory.path().join("health.db");
    let media = directory.path().join("media");
    let pool = crate::db::pool::create_pool(&format!("sqlite://{}?mode=rwc", database.display()))
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "health-secret".into(), 1);
    let engine = Arc::new(crate::engine::chat_engine::ChatEngine::new(
        pool.clone(),
        auth.clone(),
        "health-replay",
        4_000,
        100,
    ));
    let key = directory.path().join("external.key");
    std::fs::write(&key, hex::encode([19_u8; 32])).unwrap();
    let vault = Arc::new(crate::secrets::SecretVault::load(&key).unwrap());
    engine.configure_integration_vault(vault.clone()).unwrap();
    let health = Arc::new(super::super::app_state::HealthState::default());
    let state = Arc::new(super::super::app_state::AppState {
        engine,
        db: pool,
        auth_config: crate::auth::config::AuthConfig {
            jwt_secret: "health-secret".into(),
            session_expiry_hours: 1,
            public_url: "http://localhost:3000".into(),
        },
        auth,
        atproto: super::super::atproto::AtprotoOAuth::unavailable(),
        secret_vault: vault,
        egress: Arc::new(crate::egress::EgressServices::internet().unwrap()),
        max_file_size: 1_024,
        max_media_per_user: 1_024,
        max_media_total: 4_096,
        upload_admission: Arc::new(tokio::sync::Semaphore::new(4)),
        upload_idle_timeout: Duration::from_secs(1),
        upload_total_timeout: Duration::from_secs(2),
        max_message_length: 4_000,
        admin_user_ids: Arc::from([]),
        health: health.clone(),
        shutdown: tokio_util::sync::CancellationToken::new(),
        media_dir: media,
    });
    (
        super::super::router::build_router(state.clone()),
        state,
        directory,
    )
}

#[tokio::test]
async fn media_probe_syncs_and_removes_its_file() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("media");
    probe_media_storage(&root).await.unwrap();
    assert!(root.is_dir());
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
}

#[tokio::test]
async fn metrics_snapshot_uses_only_fixed_low_cardinality_dimensions() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("metrics.db");
    let pool = crate::db::pool::create_pool(&format!("sqlite://{}?mode=rwc", database.display()))
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    let row = collect_metrics(&pool).await.unwrap();
    let mut output = String::new();
    append_database_metrics(&mut output, &row);

    assert!(output.contains(&format!(
        "concord_database_schema_version {}",
        crate::db::pool::current_schema_version()
    )));
    assert_eq!(output.matches("concord_attachments{state=").count(), 7);
    assert_eq!(output.matches("concord_external_jobs{state=").count(), 5);
    append_runtime_metrics(&mut output);
    assert_eq!(
        output
            .matches("concord_runtime_operations_total{operation=")
            .count(),
        crate::runtime_metrics::Operation::ALL.len() * 2
    );
    for operation in [
        "command_admission",
        "message_commit",
        "command_ack",
        "outbound_queue",
        "queue_overflow",
        "resync",
        "replay",
        "database_write",
        "upload",
        "job_dispatch",
        "readiness_probe",
        "metrics_collection",
        "migration",
    ] {
        assert!(output.contains(&format!("operation=\"{operation}\"")));
    }
    assert!(!output.contains("message_id"));
    assert!(!output.contains("user_id"));
    pool.close().await;
}

#[tokio::test]
async fn dependency_probe_keeps_cleanup_and_admission_owned_after_caller_cancellation() {
    let health = Arc::new(super::super::app_state::HealthState::default());
    let directory = tempdir().unwrap();
    let probe_file = directory.path().join("owned-probe");
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let work_path = probe_file.clone();
    let work_started = started.clone();
    let work_release = release.clone();
    let receiver = spawn_dependency_probe(&health, async move {
        tokio::fs::write(&work_path, b"probe").await.unwrap();
        work_started.notify_one();
        work_release.notified().await;
        tokio::fs::remove_file(work_path).await.unwrap();
        Ok(())
    })
    .unwrap();
    started.notified().await;
    drop(receiver);
    assert!(probe_file.exists());
    assert!(health.begin_dependency_probe().is_none());
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !probe_file.exists()
                && let Some(probe) = health.begin_dependency_probe()
            {
                drop(probe);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dependency_probe_timeout_does_not_release_ownership_early() {
    let health = Arc::new(super::super::app_state::HealthState::default());
    let release = Arc::new(tokio::sync::Notify::new());
    let work_release = release.clone();
    let receiver = spawn_dependency_probe(&health, async move {
        work_release.notified().await;
        Ok(())
    })
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), receiver)
            .await
            .is_err()
    );
    assert!(health.begin_dependency_probe().is_none());
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(probe) = health.begin_dependency_probe() {
                drop(probe);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn metrics_collection_finishes_and_caches_after_waiter_cancellation() {
    let health = Arc::new(super::super::app_state::HealthState::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let work_started = started.clone();
    let work_release = release.clone();
    let receiver = spawn_metrics_work(&health, async move {
        work_started.notify_one();
        work_release.notified().await;
        (StatusCode::OK, "owned metrics".into())
    })
    .unwrap();
    started.notified().await;
    drop(receiver);
    assert!(health.begin_metrics_collection().is_none());
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if health.cached_metrics(Duration::from_secs(1)).is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        health.cached_metrics(Duration::from_secs(1)),
        Some((200, "owned metrics".into()))
    );
    assert!(health.begin_metrics_collection().is_some());
}

#[tokio::test]
async fn readiness_requires_listeners_admission_schema_write_and_synced_media() {
    let (router, state, _directory) = application().await;
    let request = || {
        Request::builder()
            .uri("/health/ready")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );

    state.health.set_web_listener_bound(true);
    state.health.set_irc_listener_bound(true);
    state.health.set_ready(true);
    assert_eq!(
        router.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::OK
    );

    std::fs::remove_dir(&state.media_dir).unwrap();
    std::fs::write(&state.media_dir, b"not a directory").unwrap();
    assert_eq!(
        router.oneshot(request()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn metrics_route_requires_a_current_system_admin_session() {
    let (router, state, _directory) = application().await;
    for (id, username, is_admin) in [("member", "member", false), ("monitor", "monitor", true)] {
        sqlx::query("INSERT INTO users(id,username,is_system_admin) VALUES(?,?,?)")
            .bind(id)
            .bind(username)
            .bind(is_admin)
            .execute(&state.db)
            .await
            .unwrap();
    }
    let member_session = state.auth.issue_web_session("member").await.unwrap().0;
    let monitor_session = state.auth.issue_web_session("monitor").await.unwrap().0;
    let request = |session: Option<&str>| {
        let mut builder = Request::builder().uri("/metrics");
        if let Some(session) = session {
            builder = builder.header(
                axum::http::header::COOKIE,
                format!("concord_session={session}"),
            );
        }
        builder.body(Body::empty()).unwrap()
    };

    let unauthenticated = router.clone().oneshot(request(None)).await.unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    let ordinary_member = router
        .clone()
        .oneshot(request(Some(&member_session)))
        .await
        .unwrap();
    assert_eq!(ordinary_member.status(), StatusCode::FORBIDDEN);

    let collection = state.health.begin_metrics_collection().unwrap();
    let busy = router
        .clone()
        .oneshot(request(Some(&monitor_session)))
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(collection);

    let response = router
        .clone()
        .oneshot(request(Some(&monitor_session)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[axum::http::header::CONTENT_TYPE],
        super::METRICS_CONTENT_TYPE
    );
    let body = String::from_utf8(
        to_bytes(response.into_body(), 32 * 1_024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("concord_metrics_collection_success 1"));
    assert!(body.contains("concord_runtime_operations_total"));
    assert!(body.contains("concord_health_component_ready{component=\"web_listener\"} 0"));
    assert!(!body.contains("health-secret"));

    let collection = state.health.begin_metrics_collection().unwrap();
    let cached = router
        .oneshot(request(Some(&monitor_session)))
        .await
        .unwrap();
    assert_eq!(cached.status(), StatusCode::OK);
    drop(collection);
}
