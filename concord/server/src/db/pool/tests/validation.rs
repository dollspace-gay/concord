use super::*;

#[tokio::test]
async fn noncontiguous_history_is_rejected_without_metadata_changes() {
    let before = crate::runtime_metrics::snapshot();
    let migration_index = crate::runtime_metrics::Operation::Migration as usize;
    let pool = create_pool("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(MIGRATIONS[0].sql)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE schema_version(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL DEFAULT (datetime('now')))").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO schema_version(version) VALUES (2),(4)")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        run_migrations(&pool)
            .await
            .unwrap_err()
            .to_string()
            .contains("not contiguous")
    );
    let mut conn = pool.acquire().await.unwrap();
    assert!(
        !object_exists(&mut conn, "table", "migration_metadata")
            .await
            .unwrap()
    );
    let after = crate::runtime_metrics::snapshot();
    assert!(after.failed[migration_index] > before.failed[migration_index]);
}

#[tokio::test]
async fn checksum_drift_is_rejected() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("UPDATE migration_metadata SET checksum_sha256=? WHERE version=1")
        .bind("0".repeat(64))
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        run_migrations(&pool)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
}

#[tokio::test]
async fn full_schema_drift_is_rejected_without_upgrade_mutation() {
    let pool = historical_fixture(16).await;
    sqlx::query("ALTER TABLE reactions ADD COLUMN unrecognized_drift TEXT")
        .execute(&pool)
        .await
        .unwrap();
    let before: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='table' AND name='reactions'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let error = run_migrations(&pool).await.unwrap_err().to_string();
    assert!(error.contains("schema fingerprint"), "{error}");
    let after: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='table' AND name='reactions'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, before);
    assert!(
        !object_exists(
            &mut pool.acquire().await.unwrap(),
            "table",
            "migration_metadata"
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn operator_repair_rejects_unrecognized_schema_without_mutation() {
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
            id,channel_id,target_type,target_id \
         ) VALUES('override','channel','user','legacy-uuid')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ALTER TABLE roles ADD COLUMN unrecognized_drift TEXT")
        .execute(&pool)
        .await
        .unwrap();

    let error = repair_user_override(&pool, "override", "mapped", "evidence")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("schema fingerprint"), "{error}");
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "legacy-uuid"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        14
    );
}
