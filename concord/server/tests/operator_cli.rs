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

#[test]
fn publication_inventory_and_reconcile_requeue_an_eligible_failed_record() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("atproto-publication-inventory")
            .status()
            .unwrap()
            .success()
    );
    let loaded = concord_server::config::ServerConfig::load(config).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('author','author')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','author')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,atproto_publication_enabled) VALUES('channel','server','#public',1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','author','author','publish me')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO oauth_accounts(id,user_id,provider,provider_id,credential_state) VALUES('account','author','atproto','did:plc:author','active')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO atproto_publication_grants(user_id,channel_id,enabled,grant_version) VALUES('author','channel',1,3)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO atproto_publications(id,user_id,source_message_id,source_version,destination,collection,record_key,status,safe_error_code) VALUES('publication','author','message',1,'did:plc:author','app.bsky.feed.post','stable','failed','provider_unavailable')").execute(&pool).await.unwrap();
    });
    let inventory = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .arg("atproto-publication-inventory")
        .output()
        .unwrap();
    assert!(inventory.status.success());
    let inventory = String::from_utf8(inventory.stdout).unwrap();
    assert!(inventory.contains("\"id\":\"publication\""));
    assert!(inventory.contains("\"status\":\"failed\""));
    assert!(inventory.contains("provider_unavailable"));

    let reconcile = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["atproto-publication-reconcile", "publication"])
        .output()
        .unwrap();
    assert!(
        reconcile.status.success(),
        "{}",
        String::from_utf8_lossy(&reconcile.stderr)
    );
    assert!(
        String::from_utf8_lossy(&reconcile.stdout)
            .contains("publication_requeued=publication status=pending")
    );
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url).await.unwrap();
        let state: (String, Option<String>) = sqlx::query_as("SELECT status,safe_error_code FROM atproto_publications WHERE id='publication'").fetch_one(&pool).await.unwrap();
        assert_eq!(state, ("pending".into(), None));
        let job: (String, String) = sqlx::query_as("SELECT operation_type,destination_grant FROM external_jobs WHERE resource_id='publication'").fetch_one(&pool).await.unwrap();
        assert_eq!(job, ("atproto_publish".into(), "atproto-user:author:3".into()));
    });
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blocked_v14_override_has_an_operator_inventory_repair_and_upgrade_journey() {
    const LEGACY_TO_14: &[&str] = &[
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_servers.sql"),
        include_str!("../migrations/003_messaging_enhancements.sql"),
        include_str!("../migrations/004_media_files.sql"),
        include_str!("../migrations/005_atproto_blob_storage.sql"),
        include_str!("../migrations/006_server_config.sql"),
        include_str!("../migrations/007_organization_permissions.sql"),
        include_str!("../migrations/008_user_experience.sql"),
        include_str!("../migrations/009_threads_pinning.sql"),
        include_str!("../migrations/010_moderation.sql"),
        include_str!("../migrations/011_community.sql"),
        include_str!("../migrations/012_integrations.sql"),
        include_str!("../migrations/013_atproto_integration.sql"),
        include_str!("../migrations/014_user_id_to_did.sql"),
    ];
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    let loaded = concord_server::config::ServerConfig::load_for_recovery(config).unwrap();
    let database_path = root.join("data/concord.db");
    if database_path.exists() {
        fs::remove_file(&database_path).unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE schema_version(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL DEFAULT (datetime('now')))")
            .execute(&mut *connection).await.unwrap();
        for (index, script) in LEGACY_TO_14.iter().enumerate() {
            sqlx::raw_sql(*script)
                .execute(&mut *connection)
                .await
                .unwrap();
            sqlx::query("INSERT OR IGNORE INTO schema_version(version) VALUES(?)")
                .bind((index + 1) as i64)
                .execute(&mut *connection)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('mapped','mapped')")
            .execute(&mut *connection).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&mut *connection).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#safe')")
            .execute(&mut *connection).await.unwrap();
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits,deny_bits) VALUES('override','channel','user','legacy-uuid',17,4)")
            .execute(&mut *connection).await.unwrap();
        drop(connection);
        pool.close().await;
    });

    let inventory = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .arg("migration-inventory")
        .output()
        .unwrap();
    assert!(!inventory.status.success());
    let inventory = String::from_utf8(inventory.stdout).unwrap();
    assert!(inventory.contains("unresolved_user_override"));
    assert!(inventory.contains("override"));

    let repair = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "migration-repair-user-override",
            "--override-id",
            "override",
            "--target-user-id",
            "mapped",
            "--evidence",
            "ticket MIG-14 verified ownership",
        ])
        .output()
        .unwrap();
    assert!(
        repair.status.success(),
        "{}",
        String::from_utf8_lossy(&repair.stderr)
    );
    assert!(String::from_utf8_lossy(&repair.stdout).contains("legacy-uuid"));
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("secrets-migrate")
            .status()
            .unwrap()
            .success()
    );
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "mapped"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM migration_repair_log \
                 WHERE repair_kind='post014_user_override' \
                   AND details LIKE '%MIG-14%'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_version")
                .fetch_one(&pool)
                .await
                .unwrap(),
            concord_server::db::pool::current_schema_version()
        );
    });
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stopped_operator_admin_credential_migration_and_job_recovery_are_audited() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    let loaded = concord_server::config::ServerConfig::load_for_recovery(config).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        concord_server::db::pool::run_migrations(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users(id,username,is_system_admin) VALUES \
             ('did:plc:old','old',1),('did:plc:new','new',0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES \
             ('old-at','did:plc:old','atproto','did:plc:old'), \
             ('new-at','did:plc:new','atproto','did:plc:new')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth_credentials(id,user_id,kind,scopes,expires_at) \
             VALUES('new-session','did:plc:new','web_session','human',unixepoch()+3600)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let access_hash = hex::encode(Sha256::digest(b"delegated-access"));
        let refresh_hash = hex::encode(Sha256::digest(b"delegated-refresh"));
        sqlx::query(
            "INSERT INTO oauth2_apps( \
               id,name,owner_id,client_secret,redirect_uris,scopes,is_public, \
               client_type,credential_state) \
             VALUES('app','App','did:plc:old','','[\"https://app.example/callback\"]', \
                    'identify',1,'public','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_grants( \
               id,app_id,user_id,server_id,resource_key,scopes,state) \
             VALUES('grant','app','did:plc:new',NULL,'','identify','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_tokens( \
               id,grant_id,token_family_id,access_token_hash,refresh_token_hash,scopes, \
               access_expires_at,refresh_expires_at) \
             VALUES('delegated-token','grant','family',?,?,'identify', \
                    datetime('now','+1 hour'),datetime('now','+1 day'))",
        )
        .bind(access_hash)
        .bind(refresh_hash)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_codes( \
               id,code_hash,app_id,user_id,redirect_uri,scopes,code_challenge, \
               code_challenge_method,expires_at) \
             VALUES('code','code-hash','app','did:plc:new','https://app.example/callback', \
                    'identify','challenge','S256',datetime('now','+5 minutes'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth2_consent_requests( \
               id_hash,app_id,user_id,redirect_uri,scopes,code_challenge,expires_at) \
             VALUES('consent','app','did:plc:new','https://app.example/callback','identify', \
                    'challenge',datetime('now','+5 minutes'))",
        )
        .execute(&pool)
        .await
        .unwrap();
    });

    let original_config = fs::read_to_string(config).unwrap();
    assert!(original_config.contains("admin_user_ids = []"));
    fs::write(
        config,
        original_config.replace("admin_user_ids = []", "admin_user_ids = [\"did:plc:old\"]"),
    )
    .unwrap();
    let refused = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "admin-transfer",
            "--from-user-id",
            "did:plc:old",
            "--to-user-id",
            "did:plc:new",
            "--reason",
            "planned administrator transfer",
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("remove did:plc:old from admin.admin_user_ids")
    );

    fs::write(config, &original_config).unwrap();
    let transfer = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "admin-transfer",
            "--from-user-id",
            "did:plc:old",
            "--to-user-id",
            "did:plc:new",
            "--reason",
            "planned administrator transfer",
        ])
        .output()
        .unwrap();
    assert!(
        transfer.status.success(),
        "{}",
        String::from_utf8_lossy(&transfer.stderr)
    );
    let inventory = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .arg("admin-inventory")
        .output()
        .unwrap();
    assert!(inventory.status.success());
    assert!(String::from_utf8_lossy(&inventory.stdout).contains("did:plc:new"));

    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let admins: (i64, i64) = sqlx::query_as(
            "SELECT \
             (SELECT is_system_admin FROM users WHERE id='did:plc:old'), \
             (SELECT is_system_admin FROM users WHERE id='did:plc:new')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(admins, (0, 1));
        let current = concord_server::config::ServerConfig::load(config).unwrap();
        assert!(
            !concord_server::config::ensure_configured_admin(
                &pool,
                "did:plc:old",
                &current.admin.admin_user_ids,
            )
            .await
            .unwrap()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT is_system_admin FROM users WHERE id='did:plc:old'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );

        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES('server','Server','did:plc:new')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhooks( \
               id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state) \
             VALUES('hook','server','channel','Hook','outgoing','legacy-token', \
                    'https://receiver.example/hook','did:plc:new','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO external_jobs( \
               id,deduplication_key,operation_type,resource_id,resource_version, \
               destination_grant,payload_json,state,attempt_count,safe_error_code) \
             VALUES('job','job-key','webhook_delivery','delivery',1,'webhook:hook:1', \
                    '{\"channel_id\":\"channel\"}','failed',8,'receiver_unavailable')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhook_deliveries( \
               id,webhook_id,external_job_id,delivery_id,event_type,event_version,payload_json,state) \
             VALUES('delivery-row','hook','job','delivery','webhook_test',1, \
                    '{\"channel_id\":\"channel\"}','failed')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO external_jobs( \
               id,deduplication_key,operation_type,resource_id,resource_version, \
               destination_grant,payload_json,state) \
             VALUES('at-job','at-job-key','atproto_publish','publication',1, \
                    'atproto-user:user:1','{}','failed')",
        )
        .execute(&pool)
        .await
        .unwrap();
    });

    for command in ["migration-status", "migration-apply"] {
        let result = fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg(command)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}: {}",
            command,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let jobs = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["jobs-inspect", "--state", "failed", "--limit", "10"])
        .output()
        .unwrap();
    assert!(jobs.status.success());
    let jobs = String::from_utf8(jobs.stdout).unwrap();
    assert!(jobs.contains("\"id\":\"job\""));
    assert!(!jobs.contains("legacy-token"));
    assert!(!jobs.contains("destination_grant"));
    assert!(!jobs.contains("payload_json"));

    let retry = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["job-retry", "job", "--reason", "receiver repaired"])
        .output()
        .unwrap();
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let at_retry = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["job-retry", "at-job", "--reason", "provider repaired"])
        .output()
        .unwrap();
    assert!(!at_retry.status.success());
    assert!(String::from_utf8_lossy(&at_retry.stderr).contains("atproto-publication-reconcile"));

    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_operator_credential_revoke \
             BEFORE UPDATE OF state ON oauth2_grants \
             WHEN OLD.user_id='did:plc:new' AND NEW.state='revoked' \
             BEGIN SELECT RAISE(ABORT,'injected credential revocation failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
    });
    let failed_credentials = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "credential-revoke-all",
            "--user-id",
            "did:plc:new",
            "--reason",
            "fault injection",
        ])
        .output()
        .unwrap();
    assert!(!failed_credentials.status.success());
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let unchanged: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM auth_credentials WHERE user_id='did:plc:new' AND revoked_at IS NULL), \
             (SELECT count(*) FROM oauth2_tokens WHERE grant_id='grant' AND revoked_at IS NULL), \
             (SELECT count(*) FROM oauth2_grants WHERE id='grant' AND state='active'), \
             (SELECT count(*) FROM oauth2_codes WHERE user_id='did:plc:new' AND consumed_at IS NULL), \
             (SELECT count(*) FROM oauth2_consent_requests WHERE user_id='did:plc:new' AND consumed_at IS NULL), \
             (SELECT count(*) FROM operator_audit_log WHERE action_type='credential_revoke_all')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unchanged, (1, 1, 1, 1, 1, 0));
        sqlx::query("DROP TRIGGER fail_operator_credential_revoke")
            .execute(&pool)
            .await
            .unwrap();
    });

    let credentials = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "credential-revoke-all",
            "--user-id",
            "did:plc:new",
            "--reason",
            "lost browser credential",
        ])
        .output()
        .unwrap();
    assert!(credentials.status.success());
    assert!(String::from_utf8_lossy(&credentials.stdout).contains("credentials_revoked=5"));

    runtime.block_on(async {
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode, header};

        let issuer = restarted_issuer(&loaded).await;
        let access = issuer
            .clone()
            .oneshot(
                Request::get("/api/oauth/userinfo")
                    .header(header::AUTHORIZATION, "Bearer delegated-access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(access.status(), StatusCode::UNAUTHORIZED);

        let refresh_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("client_id", "app")
            .append_pair("refresh_token", "delegated-refresh")
            .finish();
        let refresh = issuer
            .oneshot(
                Request::post("/api/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(refresh_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refresh.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(refresh.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("invalid_grant"));
    });

    let recovered = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args([
            "admin-recover",
            "--user-id",
            "did:plc:old",
            "--reason",
            "documented local recovery",
        ])
        .output()
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );

    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url)
            .await
            .unwrap();
        let job_states: (String, String) = sqlx::query_as(
            "SELECT \
             (SELECT state FROM external_jobs WHERE id='job'), \
             (SELECT state FROM webhook_deliveries WHERE delivery_id='delivery')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(job_states, ("pending".into(), "pending".into()));
        assert!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT revoked_at FROM auth_credentials WHERE id='new-session'",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
            .is_some()
        );
        let delegated: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT g.state,t.revoked_at,c.consumed_at,r.consumed_at \
                 FROM oauth2_grants g JOIN oauth2_tokens t ON t.grant_id=g.id \
                 JOIN oauth2_codes c ON c.user_id=g.user_id \
                 JOIN oauth2_consent_requests r ON r.user_id=g.user_id \
                 WHERE g.id='grant'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(delegated.0, "revoked");
        assert!(delegated.1.is_some());
        assert!(delegated.2.is_some());
        assert!(delegated.3.is_some());
        let mut actions: Vec<String> =
            sqlx::query_scalar("SELECT action_type FROM operator_audit_log")
                .fetch_all(&pool)
                .await
                .unwrap();
        actions.sort();
        assert_eq!(
            actions,
            vec![
                "admin_recovery",
                "admin_transfer",
                "credential_revoke_all",
                "external_job_retry",
            ]
        );
    });
    fs::remove_dir_all(root).unwrap();
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

#[test]
fn key_rotation_activates_a_durable_replacement_and_preserves_recovery_key() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("secrets-migrate")
            .status()
            .unwrap()
            .success()
    );
    let active = root.join("data/secrets/external-credentials.key");
    let old = fs::read(&active).unwrap();
    let replacement = root.join("replacement.key");
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["key-init", "--key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    let replacement_bytes = fs::read(&replacement).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .args(["secrets-rotate", "--new-key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&active).unwrap(), replacement_bytes);
    let second_replacement = root.join("second-replacement.key");
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["key-init", "--key-file"])
            .arg(&second_replacement)
            .status()
            .unwrap()
            .success()
    );
    let second_bytes = fs::read(&second_replacement).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .args(["secrets-rotate", "--new-key-file"])
            .arg(&second_replacement)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&active).unwrap(), second_bytes);
    let siblings: Vec<_> = fs::read_dir(active.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        siblings
            .iter()
            .any(|name| name.starts_with("external-credentials.previous-"))
    );
    assert!(
        siblings
            .iter()
            .any(|name| name.starts_with("external-credentials.replacement-"))
    );
    assert_ne!(old, replacement_bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn media_inventory_remains_available_when_provider_key_is_lost_and_honors_exclusion() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("secrets-migrate")
            .status()
            .unwrap()
            .success()
    );
    fs::remove_file(root.join("data/secrets/external-credentials.key")).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("media-inventory")
            .status()
            .unwrap()
            .success()
    );

    let lock_path = root.join("data/concord.db.concord-maintenance.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    FileExt::try_lock_exclusive(&lock).unwrap();
    let blocked = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .arg("media-inventory")
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("maintenance"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "storage-fault-injection")]
fn committed_rotation_recovers_after_sigkill_using_durable_replacement() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    populate_encrypted_rotation_fixture(config, &fixture.binaries.operator);
    let active = root.join("data/secrets/external-credentials.key");
    let replacement = root.join("replacement.key");
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["key-init", "--key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    let replacement_bytes = fs::read(&replacement).unwrap();
    let barrier = root.join("rotation-barrier");
    let mut interrupted = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["secrets-rotate", "--new-key-file"])
        .arg(&replacement)
        .env("CONCORD_ROTATION_TEST_BARRIER", &barrier)
        .spawn()
        .unwrap();
    wait_for_or_kill(
        &mut interrupted,
        &std::path::PathBuf::from(format!("{}.database-committed", barrier.display())),
    );
    interrupted.kill().unwrap();
    interrupted.wait().unwrap();
    fs::remove_file(&replacement).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .args(["secrets-rotate", "--new-key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&active).unwrap(), replacement_bytes);

    let loaded = concord_server::config::ServerConfig::load(config).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url).await.unwrap();
        let phase: String = sqlx::query_scalar("SELECT phase FROM credential_rotation_state WHERE singleton=1").fetch_one(&pool).await.unwrap();
        assert_eq!(phase, "activated");
        let distinct: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT credential_key_id) FROM oauth_accounts WHERE credential_state='active'").fetch_one(&pool).await.unwrap();
        assert_eq!(distinct, 1);
        let pending_key: String = sqlx::query_scalar("SELECT credential_key_id FROM pending_atproto_oauth WHERE state_hash='pending-state'").fetch_one(&pool).await.unwrap();
        let vault = concord_server::secrets::SecretVault::load(&active).unwrap();
        assert_eq!(pending_key, vault.key_id());
        let credentials = concord_server::db::queries::users::get_atproto_credentials_encrypted(
            &pool, &vault, "at-user",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(credentials.access_token, "access");
        assert_eq!(credentials.refresh_token, "refresh");
        assert_eq!(credentials.dpop_private_key, "dpop");
        let pending_ciphertext: String = sqlx::query_scalar("SELECT credential_ciphertext FROM pending_atproto_oauth WHERE state_hash='pending-state'").fetch_one(&pool).await.unwrap();
        assert_eq!(vault.decrypt("atproto:pending:pending-state", &pending_ciphertext, &pending_key).unwrap(), b"pending-secret");
        let signing: String = sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'").fetch_one(&pool).await.unwrap();
        let signing: serde_json::Value = serde_json::from_str(&signing).unwrap();
        assert_eq!(vault.decrypt("atproto:client-signing-key", signing["ciphertext"].as_str().unwrap(), signing["key_id"].as_str().unwrap()).unwrap(), b"signing-secret");
    });
    fs::remove_dir_all(root).unwrap();
}
