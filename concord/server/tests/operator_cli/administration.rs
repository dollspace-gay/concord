use super::*;

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
        include_str!("../../migrations/001_initial.sql"),
        include_str!("../../migrations/002_servers.sql"),
        include_str!("../../migrations/003_messaging_enhancements.sql"),
        include_str!("../../migrations/004_media_files.sql"),
        include_str!("../../migrations/005_atproto_blob_storage.sql"),
        include_str!("../../migrations/006_server_config.sql"),
        include_str!("../../migrations/007_organization_permissions.sql"),
        include_str!("../../migrations/008_user_experience.sql"),
        include_str!("../../migrations/009_threads_pinning.sql"),
        include_str!("../../migrations/010_moderation.sql"),
        include_str!("../../migrations/011_community.sql"),
        include_str!("../../migrations/012_integrations.sql"),
        include_str!("../../migrations/013_atproto_integration.sql"),
        include_str!("../../migrations/014_user_id_to_did.sql"),
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
