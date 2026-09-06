use std::fs;

#[cfg(feature = "storage-fault-injection")]
use std::time::{Duration, Instant};

use fs2::FileExt;

use sha2::{Digest, Sha256};

use std::sync::Arc;

use tower::ServiceExt;

use uuid::Uuid;

mod common;

use common::VerifiedBinary;

struct TestBinaries {
    server: VerifiedBinary,
    operator: VerifiedBinary,
}

impl TestBinaries {
    fn copy_to(root: &std::path::Path) -> Self {
        Self {
            server: VerifiedBinary::copy_from(
                std::path::Path::new(env!("CARGO_BIN_EXE_concord-server")),
                root.join("concord-server"),
            ),
            operator: VerifiedBinary::copy_from(
                std::path::Path::new(env!("CARGO_BIN_EXE_concord_operator")),
                root.join("concord-operator"),
            ),
        }
    }
}

struct Fixture {
    root: std::path::PathBuf,
    config: std::path::PathBuf,
    binaries: TestBinaries,
}

fn initialized() -> Fixture {
    let root = std::env::temp_dir().join(format!("concord-operator-{}", Uuid::new_v4()));
    let config = root.join("concord.toml");
    let binaries = TestBinaries::copy_to(&root.join("test-bin"));
    assert!(
        binaries
            .server
            .command()
            .args(["init", "--config"])
            .arg(&config)
            .status()
            .unwrap()
            .success()
    );
    Fixture {
        root,
        config,
        binaries,
    }
}

async fn restarted_issuer(config: &concord_server::config::ServerConfig) -> axum::Router {
    use concord_server::auth::authority::AuthService;
    use concord_server::engine::chat_engine::ChatEngine;
    use concord_server::web::app_state::{AppState, HealthState};
    use concord_server::web::atproto::AtprotoOAuth;
    use tokio_util::sync::CancellationToken;

    let pool = concord_server::db::pool::create_pool(&config.database.url)
        .await
        .unwrap();
    let auth_config = config.to_auth_config();
    let auth = AuthService::new(
        pool.clone(),
        auth_config.jwt_secret.clone(),
        auth_config.session_expiry_hours,
    );
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth.clone(),
        &auth_config.jwt_secret,
        config.storage.max_message_length,
        config.storage.max_file_size_mb,
    ));
    let vault = Arc::new(
        concord_server::secrets::SecretVault::load(&config.auth.external_credentials_key_file)
            .unwrap(),
    );
    engine.configure_integration_vault(vault.clone()).unwrap();
    let atproto = AtprotoOAuth::load_or_create(&pool, &vault).await.unwrap();
    concord_server::web::router::build_router(Arc::new(AppState {
        engine,
        db: pool,
        auth_config,
        auth,
        atproto,
        secret_vault: vault,
        egress: Arc::new(concord_server::egress::EgressServices::internet().unwrap()),
        max_file_size: config.storage.max_file_size_mb * 1024 * 1024,
        max_media_per_user: config.storage.max_media_per_user_mb * 1024 * 1024,
        max_media_total: config.storage.max_media_total_mb * 1024 * 1024,
        upload_admission: Arc::new(tokio::sync::Semaphore::new(1)),
        upload_idle_timeout: std::time::Duration::from_secs(1),
        upload_total_timeout: std::time::Duration::from_secs(2),
        max_message_length: config.storage.max_message_length,
        admin_user_ids: config.admin.admin_user_ids.clone().into(),
        health: Arc::new(HealthState::default()),
        shutdown: CancellationToken::new(),
        media_dir: config.storage.media_dir.clone(),
    }))
}

#[cfg(feature = "storage-fault-injection")]
fn wait_for_or_kill(child: &mut std::process::Child, path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(feature = "storage-fault-injection")]
fn populate_encrypted_rotation_fixture(config_path: &std::path::Path, operator: &VerifiedBinary) {
    let config = concord_server::config::ServerConfig::load(config_path).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&config.database.url)
            .await
            .unwrap();
        concord_server::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('at-user','at-user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO oauth_accounts(user_id,provider,provider_id,access_token,refresh_token,dpop_private_key,pds_url,token_expires_at,credential_state) VALUES('at-user','atproto','did:plc:test','access','refresh','dpop','https://pds.example','2999-01-01T00:00:00Z','legacy_plaintext')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT OR REPLACE INTO server_config(key,value) VALUES('atproto_signing_key','signing-secret')")
            .execute(&pool).await.unwrap();
        let vault = concord_server::secrets::SecretVault::load(
            &config.auth.external_credentials_key_file,
        )
        .unwrap();
        let pending = vault
            .encrypt("atproto:pending:pending-state", b"pending-secret")
            .unwrap();
        sqlx::query("INSERT INTO pending_atproto_oauth(state_hash,credential_key_id,credential_ciphertext,created_at,expires_at) VALUES('pending-state',?,?,datetime('now'),datetime('now','+1 hour'))")
            .bind(vault.key_id()).bind(pending).execute(&pool).await.unwrap();
    });
    assert!(
        operator
            .command()
            .args(["--config"])
            .arg(config_path)
            .arg("secrets-migrate")
            .status()
            .unwrap()
            .success()
    );
}

#[path = "operator_cli/administration.rs"]
mod administration;
#[path = "operator_cli/credentials.rs"]
mod credentials;
#[path = "operator_cli/key_rotation.rs"]
mod key_rotation;
#[path = "operator_cli/media.rs"]
mod media;
