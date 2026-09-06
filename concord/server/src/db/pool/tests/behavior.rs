use super::*;

#[test]
fn latest_schema_version_matches_registered_migration_tail() {
    assert_eq!(
        LATEST_SCHEMA_VERSION,
        MIGRATIONS.last().expect("at least one migration").version
    );
}

#[tokio::test]
async fn every_historical_version_upgrades_and_preserves_rows() {
    for source in 1..=16 {
        let pool = historical_fixture(source).await;
        run_migrations(&pool)
            .await
            .unwrap_or_else(|error| panic!("version {source} failed to upgrade: {error}"));
        let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
            .bind(format!("legacy-user-{source}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(username, format!("legacy{source}"));
        let adopted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM migration_metadata WHERE version<=? AND provenance='adopted_release_effects'")
            .bind(source).fetch_one(&pool).await.unwrap();
        assert_eq!(adopted, source, "source version {source}");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM schema_version")
                .fetch_one(&pool)
                .await
                .unwrap(),
            current_schema_version()
        );
    }
}

#[tokio::test]
async fn blocked_v14_override_can_be_audited_then_fully_upgraded() {
    let pool = historical_fixture(14).await;
    sqlx::query("INSERT INTO users(id,username) VALUES('mapped','mapped')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO servers(id,name,owner_id) \
         VALUES('server','Server','legacy-user-14')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#safe')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO channel_permission_overrides( \
            id,channel_id,target_type,target_id,allow_bits,deny_bits \
         ) VALUES('override','channel','user','legacy-uuid',9,2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migration_preflight(&pool).await.unwrap().is_blocked());
    repair_user_override(&pool, "override", "mapped", "MIG-14 ownership evidence")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        17
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM migration_repair_log \
             WHERE repair_kind='post014_user_override' AND outcome='operator_mapped'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    run_migrations(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        current_schema_version()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "mapped"
    );
}

#[tokio::test]
async fn notification_duplicates_are_exported_deduplicated_and_constrained() {
    let pool = historical_fixture(26).await;
    sqlx::query(
        "INSERT INTO notification_settings( \
            id,user_id,level,updated_at \
         ) VALUES \
            ('older','legacy-user-26','all','2025-01-01 00:00:00'), \
            ('newer','legacy-user-26','mentions','2025-02-01 00:00:00')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let report = migration_preflight(&pool).await.unwrap();
    let duplicate = report
        .findings
        .iter()
        .find(|finding| finding.code == "duplicate_notification_scope")
        .unwrap();
    assert!(!duplicate.blocks_upgrade);

    run_migrations(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM notification_settings WHERE user_id='legacy-user-26'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "newer"
    );
    let export: String = sqlx::query_scalar(
        "SELECT details FROM migration_repair_log \
         WHERE repair_kind='duplicate_notification_scope'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(export.contains("older"));
    assert!(export.contains("newer"));
    assert!(export.contains("latest valid updated_at"));
    assert!(
        sqlx::query(
            "INSERT INTO notification_settings(id,user_id,level) \
             VALUES('duplicate','legacy-user-26','none')"
        )
        .execute(&pool)
        .await
        .is_err()
    );
}

#[test]
fn schema_normalization_preserves_literal_semantics() {
    assert_ne!(
        normalize_schema_sql(Some("CREATE TABLE t(v TEXT DEFAULT 'A B')".into())),
        normalize_schema_sql(Some("create table t(v text default 'a b')".into()))
    );
    assert_ne!(
        normalize_schema_sql(Some("CREATE TABLE t(v TEXT DEFAULT 'a b')".into())),
        normalize_schema_sql(Some("CREATE TABLE t(v TEXT DEFAULT 'ab')".into()))
    );
}
