use std::fs;
#[cfg(feature = "storage-fault-injection")]
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use concord_server::auth::authority::AuthService;
use concord_server::engine::messaging::{ContentFormat, MessagingService, SendMessageCommand};

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

struct Harness {
    binaries: TestBinaries,
}

impl Harness {
    fn new(root: &std::path::Path) -> Self {
        Self {
            binaries: TestBinaries::copy_to(&root.join("test-bin")),
        }
    }

    fn initialize(&self, root: &std::path::Path) -> std::path::PathBuf {
        fs::create_dir_all(root).unwrap();
        let config = root.join("concord.toml");
        assert!(
            self.binaries
                .server
                .command()
                .args(["init", "--config"])
                .arg(&config)
                .status()
                .unwrap()
                .success()
        );
        config
    }

    fn run(&self, config: &std::path::Path, args: &[&str]) -> std::process::Output {
        self.binaries
            .operator
            .command()
            .arg("--config")
            .arg(config)
            .args(args)
            .output()
            .unwrap()
    }
}

fn empty_initialized_restore_paths(config: &concord_server::config::ServerConfig) {
    let database = std::path::Path::new(
        config
            .database
            .url
            .trim_start_matches("sqlite:")
            .split('?')
            .next()
            .unwrap(),
    );
    if database.exists() {
        fs::remove_file(database).unwrap();
    }
    if config.storage.media_dir.exists() {
        fs::remove_dir_all(&config.storage.media_dir).unwrap();
        fs::create_dir_all(&config.storage.media_dir).unwrap();
    }
    fs::remove_file(&config.auth.external_credentials_key_file).unwrap();
}

fn restore_operation_record(output: &[u8]) -> serde_json::Value {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|record| record["kind"] == "concord_operator_operation")
        .expect("restore command must emit a structured operation record")
}

fn assert_restore_operation_record(record: &serde_json::Value, outcome: &str) {
    assert_eq!(record["kind"], "concord_operator_operation");
    assert_eq!(record["operation"], "restore");
    assert_eq!(record["outcome"], outcome);
    assert!(record["duration_seconds"].as_f64().unwrap() >= 0.0);
    let mut keys: Vec<_> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["duration_seconds", "kind", "operation", "outcome"]);
}

#[test]
fn stopped_service_backup_restores_media_credentials_and_pauses_external_work() {
    let runtime_root = std::env::temp_dir().join(format!("concord-backup-{}", Uuid::new_v4()));
    let harness = Harness::new(&runtime_root);
    let source_config = harness.initialize(&runtime_root.join("source"));
    assert!(
        harness
            .run(&source_config, &["secrets-migrate"])
            .status
            .success()
    );
    let source = concord_server::config::ServerConfig::load(&source_config).unwrap();
    let (original_generation, old_session) = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let pool = concord_server::db::pool::create_pool(&source.database.url)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        let vault = concord_server::secrets::SecretVault::load(
            &source.auth.external_credentials_key_file,
        )
        .unwrap();
        let credential = vault
            .encrypt("atproto:owner", br#"{"access_token":"preserved"}"#)
            .unwrap();
        sqlx::query("INSERT INTO oauth_accounts(id,user_id,provider,provider_id,pds_url,credential_key_id,credential_ciphertext,credential_version,credential_state) VALUES('account','owner','atproto','did:plc:owner','https://pds.example',?,?,1,'active')")
            .bind(vault.key_id()).bind(credential).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner')")
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
        sqlx::query("UPDATE conversations SET next_message_sequence=1 WHERE id=?")
            .bind(&conversation)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,conversation_id,conversation_sequence,content_format) VALUES('message','server','channel','owner','owner','preserved',?,1,'plain')")
            .bind(&conversation).execute(&pool).await.unwrap();

        let media = b"backup-media";
        let media_hash = hex::encode(Sha256::digest(media));
        let storage_key = "objects/ab/media";
        let media_path = source.storage.media_dir.join(storage_key);
        fs::create_dir_all(media_path.parent().unwrap()).unwrap();
        fs::write(&media_path, media).unwrap();
        sqlx::query("INSERT INTO attachments(id,uploader_id,message_id,filename,original_filename,content_type,file_size,conversation_id,media_state,storage_backend,storage_key,sha256,ready_at) VALUES('attachment','owner','message','media','media','application/octet-stream',?,?, 'attached','local',?,?,datetime('now'))")
            .bind(media.len() as i64).bind(&conversation).bind(storage_key).bind(&media_hash)
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) VALUES('job','job','atproto_publish','message',1,'pds:test','{}')")
            .execute(&pool).await.unwrap();
        let auth_config = source.to_auth_config();
        let auth = AuthService::new(
            pool.clone(),
            auth_config.jwt_secret,
            auth_config.session_expiry_hours,
        );
        let old_session = auth.issue_web_session("owner").await.unwrap().0;
        let generation: String = sqlx::query_scalar(
            "SELECT generation FROM database_metadata WHERE singleton=1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        pool.close().await;
        (generation, old_session)
    });

    let backup = runtime_root.join("backup");
    let created = harness.run(
        &source_config,
        &["backup-create", "--destination", backup.to_str().unwrap()],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(
        harness
            .run(
                &source_config,
                &["backup-verify", "--backup", backup.to_str().unwrap()]
            )
            .status
            .success()
    );

    let destination_config = harness.initialize(&runtime_root.join("destination"));
    let destination = concord_server::config::ServerConfig::load(&destination_config).unwrap();
    let rejected = harness.run(
        &destination_config,
        &["backup-restore", "--backup", backup.to_str().unwrap()],
    );
    assert!(!rejected.status.success());
    assert_restore_operation_record(&restore_operation_record(&rejected.stderr), "failure");
    empty_initialized_restore_paths(&destination);

    let restored = harness.run(
        &destination_config,
        &["backup-restore", "--backup", backup.to_str().unwrap()],
    );
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    let output = String::from_utf8(restored.stdout).unwrap();
    assert!(output.contains("activation_required=true"));
    assert!(output.contains("external_jobs_paused=true"));
    assert_restore_operation_record(&restore_operation_record(output.as_bytes()), "success");

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let pool = concord_server::db::pool::create_pool(&destination.database.url)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id='message'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "preserved"
        );
        assert_eq!(
            fs::read(destination.storage.media_dir.join("objects/ab/media")).unwrap(),
            b"backup-media"
        );
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_ne!(generation, original_generation);
        let (key_id, ciphertext): (String, String) = sqlx::query_as(
            "SELECT credential_key_id,credential_ciphertext FROM oauth_accounts WHERE user_id='owner'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let vault = concord_server::secrets::SecretVault::load(
            &destination.auth.external_credentials_key_file,
        )
        .unwrap();
        assert_eq!(
            vault.decrypt("atproto:owner", &ciphertext, &key_id).unwrap(),
            br#"{"access_token":"preserved"}"#
        );
        let auth_config = destination.to_auth_config();
        let auth = AuthService::new(
            pool.clone(),
            auth_config.jwt_secret,
            auth_config.session_expiry_hours,
        );
        assert!(auth.authenticate_web_session(&old_session).await.is_err());
        let actor = auth.issue_web_session("owner").await.unwrap().1;
        let messaging = MessagingService::new(pool.clone(), auth, 4_000);
        let command = || SendMessageCommand {
            request_id: "post-restore-request",
            client_message_id: "post-restore-client",
            operation_generation: None,
            conversation_id: None,
            server_id: "server",
            channel: "#general",
            content: "post restore",
            content_format: ContentFormat::Markdown,
            reply_to_id: None,
            attachment_ids: &[],
            mentions: &[],
        };
        let sent = messaging
            .send_channel_message(&actor, command())
            .await
            .unwrap();
        let replayed = messaging
            .send_channel_message(&actor, command())
            .await
            .unwrap();
        assert_eq!(sent.message_id, replayed.message_id);
        assert!(replayed.replayed);
        let epoch_types: (String, String) = sqlx::query_as(
            "SELECT typeof(issued_at),typeof(expires_at) FROM operation_generations \
             WHERE generation=(SELECT current_generation FROM operation_generation_state WHERE singleton=1)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(epoch_types, ("integer".into(), "integer".into()));
        assert_eq!(
            sqlx::query_as::<_, (String, String)>(
                "SELECT state,safe_error_code FROM external_jobs WHERE id='job'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            ("failed".into(), "restore_reconciliation_required".into())
        );
        pool.close().await;
    });

    let corrupt = backup.join("media/objects/ab/media");
    fs::write(corrupt, b"corrupt").unwrap();
    assert!(
        !harness
            .run(
                &source_config,
                &["backup-verify", "--backup", backup.to_str().unwrap()]
            )
            .status
            .success()
    );
    fs::remove_dir_all(runtime_root).unwrap();
}

#[test]
fn backup_rejects_ready_local_media_without_key_or_checksum() {
    let runtime_root =
        std::env::temp_dir().join(format!("concord-backup-invalid-media-{}", Uuid::new_v4()));
    let harness = Harness::new(&runtime_root);
    let config_path = harness.initialize(&runtime_root.join("source"));
    assert!(
        harness
            .run(&config_path, &["secrets-migrate"])
            .status
            .success()
    );
    let config = concord_server::config::ServerConfig::load(&config_path).unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let pool = concord_server::db::pool::create_pool(&config.database.url)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO attachments(id,uploader_id,filename,original_filename,content_type,file_size,media_state,storage_backend) VALUES('broken','owner','broken','broken','application/octet-stream',1,'ready','local')")
            .execute(&pool).await.unwrap();
        pool.close().await;
    });
    let backup = runtime_root.join("backup");
    let result = harness.run(
        &config_path,
        &["backup-create", "--destination", backup.to_str().unwrap()],
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("lacks a storage key"));
    assert!(!backup.exists());
    fs::remove_dir_all(runtime_root).unwrap();
}

#[cfg(feature = "storage-fault-injection")]
#[test]
fn interrupted_restore_is_fail_closed_and_resumable_before_and_during_activation() {
    let runtime_root =
        std::env::temp_dir().join(format!("concord-restore-fault-{}", Uuid::new_v4()));
    let harness = Harness::new(&runtime_root);
    let source_config = harness.initialize(&runtime_root.join("source"));
    assert!(
        harness
            .run(&source_config, &["secrets-migrate"])
            .status
            .success()
    );
    let backup = runtime_root.join("backup");
    assert!(
        harness
            .run(
                &source_config,
                &["backup-create", "--destination", backup.to_str().unwrap()]
            )
            .status
            .success()
    );

    for stage in [
        "before-rewrite",
        "after-rewrite",
        "database-activated",
        "key-activated",
    ] {
        let destination_config =
            harness.initialize(&runtime_root.join(format!("destination-{stage}")));
        let destination = concord_server::config::ServerConfig::load(&destination_config).unwrap();
        empty_initialized_restore_paths(&destination);
        let barrier = runtime_root.join(format!("barrier-{stage}"));
        let mut child = harness
            .binaries
            .operator
            .command()
            .arg("--config")
            .arg(&destination_config)
            .args(["backup-restore", "--backup"])
            .arg(&backup)
            .env("CONCORD_RESTORE_TEST_BARRIER", &barrier)
            .env("CONCORD_RESTORE_TEST_STAGE", stage)
            .spawn()
            .unwrap();
        let reached = std::path::PathBuf::from(format!("{}.{}", barrier.display(), stage));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !reached.exists() && Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(reached.exists(), "restore did not reach barrier {stage}");
        child.kill().unwrap();
        child.wait().unwrap();
        let pending =
            concord_server::operations::restore_marker_path(&destination.database.url).unwrap();
        assert!(pending.exists());

        if stage == "key-activated" {
            let blocked = harness
                .binaries
                .server
                .command()
                .args(["serve", "--config"])
                .arg(&destination_config)
                .output()
                .unwrap();
            assert!(!blocked.status.success());
            let diagnostics = format!(
                "{}{}",
                String::from_utf8_lossy(&blocked.stdout),
                String::from_utf8_lossy(&blocked.stderr)
            );
            assert!(diagnostics.contains("incomplete restore activation"));
        }

        if stage == "database-activated" {
            let original = fs::read_to_string(&destination_config).unwrap();
            let changed = original.replace(
                "media_dir = \"data/media\"",
                "media_dir = \"data/changed-media\"",
            );
            assert_ne!(changed, original);
            fs::create_dir_all(
                destination_config
                    .parent()
                    .unwrap()
                    .join("data/changed-media"),
            )
            .unwrap();
            fs::write(&destination_config, changed).unwrap();
            let rejected = harness.run(
                &destination_config,
                &["backup-restore", "--backup", backup.to_str().unwrap()],
            );
            assert!(!rejected.status.success());
            assert!(
                String::from_utf8_lossy(&rejected.stderr).contains("canonical destination config")
            );
            assert!(pending.exists());
            fs::write(&destination_config, original).unwrap();
        }

        let resumed = harness.run(
            &destination_config,
            &["backup-restore", "--backup", backup.to_str().unwrap()],
        );
        assert!(
            resumed.status.success(),
            "resume {stage}: {}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert!(!pending.exists());
        assert!(concord_server::config::ServerConfig::load(&destination_config).is_ok());
    }
    fs::remove_dir_all(runtime_root).unwrap();
}
