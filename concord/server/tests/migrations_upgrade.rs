use concord_server::db::pool::{create_pool, current_schema_version, run_migrations};
use sqlx::SqlitePool;
use uuid::Uuid;

const LEGACY: &[&str] = &[
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
    include_str!("../migrations/015_premium_for_free.sql"),
    include_str!("../migrations/016_fts_delete_trigger.sql"),
];

async fn file_fixture(version: usize) -> (SqlitePool, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("concord-migration-{}.db", Uuid::new_v4()));
    let pool = create_pool(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *conn)
        .await
        .unwrap();
    if version >= 1 {
        sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))")
            .execute(&mut *conn).await.unwrap();
    }
    for (index, script) in LEGACY.iter().take(version).enumerate() {
        sqlx::raw_sql(script).execute(&mut *conn).await.unwrap();
        if version >= 1 {
            sqlx::query("INSERT OR IGNORE INTO schema_version(version) VALUES(?)")
                .bind((index + 1) as i64)
                .execute(&mut *conn)
                .await
                .unwrap();
        }
    }
    sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
        .bind(format!("legacy-{version}"))
        .bind(format!("user{version}"))
        .execute(&mut *conn)
        .await
        .unwrap();
    if version >= 2 {
        sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
            .bind(format!("recipient-{version}"))
            .bind(format!("recipient{version}"))
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages(id,sender_id,sender_nick,target_user_id,content,created_at) VALUES(?,?,?,?,?,?)")
            .bind(format!("legacy-dm-{version}"))
            .bind(format!("legacy-{version}"))
            .bind(format!("user{version}"))
            .bind(format!("recipient-{version}"))
            .bind("offline direct message")
            .bind("2024-01-01 00:00:00")
            .execute(&mut *conn)
            .await
            .unwrap();
    }
    if version <= 13 {
        sqlx::query("INSERT INTO users(id,username) VALUES('at-user','at-user')")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES('at-account','at-user','atproto','did:plc:fixture')")
            .execute(&mut *conn).await.unwrap();
    }
    if version >= 2 {
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('fixture-server','Fixture',?)")
            .bind(format!("legacy-{version}"))
            .execute(&mut *conn)
            .await
            .unwrap();
        if version >= 7 {
            sqlx::query("INSERT INTO channels(id,server_id,name,is_private) VALUES('fixture-channel','fixture-server','#fixture',1)")
                .execute(&mut *conn).await.unwrap();
        } else {
            sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('fixture-channel','fixture-server','#fixture')")
                .execute(&mut *conn).await.unwrap();
        }
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES('fixture-message','fixture-server','fixture-channel',?,'fixture','literal &lt;tag&gt; **unicode 🦇**','2024-01-02 03:04:05')")
            .bind(format!("legacy-{version}")).execute(&mut *conn).await.unwrap();
    }
    if version >= 3 {
        sqlx::query(
            "INSERT INTO reactions(message_id,user_id,emoji) VALUES('fixture-message',?,'🦇')",
        )
        .bind(format!("legacy-{version}"))
        .execute(&mut *conn)
        .await
        .unwrap();
    }
    if version >= 4 {
        sqlx::query("INSERT INTO attachments(id,uploader_id,message_id,filename,original_filename,content_type,file_size) VALUES('fixture-attachment',?,'fixture-message','stored.bin','original.bin','application/octet-stream',7)")
            .bind(format!("legacy-{version}")).execute(&mut *conn).await.unwrap();
    }
    if version >= 7 {
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits) VALUES('fixture-override','fixture-channel','user',?,1)")
            .bind(format!("legacy-{version}")).execute(&mut *conn).await.unwrap();
        if version <= 13 {
            sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits) VALUES('at-fixture-override','fixture-channel','user','at-user',1)")
                .execute(&mut *conn).await.unwrap();
        }
    }
    if version >= 12 {
        sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('fixture-bot','fixture-bot',1)")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO webhooks(id,server_id,channel_id,name,webhook_type,token,created_by) VALUES('fixture-webhook','fixture-server','fixture-channel','Fixture hook','incoming','fixture-token',?)")
            .bind(format!("legacy-{version}")).execute(&mut *conn).await.unwrap();
    }
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);
    (pool, path)
}

#[tokio::test]
async fn populated_file_fixtures_from_every_legacy_version_upgrade_to_current() {
    for source in 1..=16 {
        let (pool, path) = file_fixture(source).await;
        run_migrations(&pool)
            .await
            .unwrap_or_else(|error| panic!("source {source}: {error}"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM schema_version")
                .fetch_one(&pool)
                .await
                .unwrap(),
            current_schema_version()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name IN ('conversation_id','conversation_sequence','entity_version')"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN ('conversations','command_receipts','event_log')"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE id=?")
                .bind(format!("legacy-{source}"))
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        if source >= 2 {
            let direct_conversation: String =
                sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                    .bind(format!("legacy-dm-{source}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM conversation_participants WHERE conversation_id=?"
                )
                .bind(&direct_conversation)
                .fetch_one(&pool)
                .await
                .unwrap(),
                2
            );
        }
        if source <= 13 {
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT user_id FROM user_aliases WHERE alias='at-user' AND alias_kind='legacy_id'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                "did:plc:fixture"
            );
            if source >= 7 {
                assert_eq!(
                    sqlx::query_scalar::<_, String>(
                        "SELECT target_id FROM channel_permission_overrides WHERE id='at-fixture-override'"
                    )
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                    "did:plc:fixture"
                );
                assert_eq!(
                    sqlx::query_scalar::<_, i64>(
                        "SELECT count(*) FROM migration_repair_log WHERE repair_kind='pre014_user_override' AND object_id='at-fixture-override' AND outcome='repaired'"
                    )
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                    1
                );
            }
        }
        if source >= 2 {
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT content FROM messages WHERE id='fixture-message'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                "literal &lt;tag&gt; **unicode 🦇**"
            );
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT created_at FROM messages WHERE id='fixture-message'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                "2024-01-02 03:04:05"
            );
        }
        if source >= 8 {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'literal'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                1
            );
        }
        if source >= 4 {
            assert_eq!(
                sqlx::query_scalar::<_, Option<String>>(
                    "SELECT message_id FROM attachments WHERE id='fixture-attachment'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                Some("fixture-message".into())
            );
        }
        if source <= 13 {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM users WHERE id='did:plc:fixture'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                1
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "ok"
        );
        assert!(
            sqlx::query_scalar::<_, i64>("SELECT row_count FROM migration_snapshots WHERE migration_version=? AND table_name='users'")
                .bind(current_schema_version())
                .fetch_one(&pool)
                .await
                .unwrap()
                > 0
        );
        pool.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
