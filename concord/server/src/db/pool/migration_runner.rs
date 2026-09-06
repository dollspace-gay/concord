use super::{
    COMPATIBILITY_FLOOR, LATEST_SCHEMA_VERSION, MIGRATIONS, MigrationError,
    MigrationPreflightReport, SqlitePool, Uuid, apply_notification_scope_repairs,
    apply_safe_repairs, capture_snapshot, checksum, info, inspect_repairs,
    migration_preflight_connection, object_exists, record_snapshot, verify_integrity,
};

/// Upgrade a recognized database using bundled whole SQL scripts under exclusive locks.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), MigrationError> {
    let mut metric =
        crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Migration);
    // The transaction owns the exclusive maintenance lock and rolls back on
    // drop, including cancellation and every early error return.
    let mut transaction = pool.begin_with("BEGIN EXCLUSIVE").await?;
    let report = match migration_preflight_connection(&mut transaction).await {
        Ok(report) => report,
        Err(error) => {
            transaction.rollback().await?;
            return Err(error);
        }
    };
    if report.is_blocked() {
        transaction.rollback().await?;
        return Err(MigrationError::Preflight(report));
    }
    let conn = &mut *transaction;
    if report.source_version == LATEST_SCHEMA_VERSION {
        verify_integrity(conn).await?;
        transaction.commit().await?;
        metric.succeed();
        return Ok(());
    }
    let snapshot = capture_snapshot(conn).await?;
    sqlx::query("PRAGMA defer_foreign_keys=ON")
        .execute(&mut *conn)
        .await?;
    if report.source_version == 0 {
        sqlx::query("CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))").execute(&mut *conn).await?;
    } else if report.source_version == 1 && !object_exists(conn, "table", "schema_version").await? {
        sqlx::query("CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))").execute(&mut *conn).await?;
        sqlx::query("INSERT INTO schema_version(version) VALUES (1)")
            .execute(&mut *conn)
            .await?;
    }
    for migration in MIGRATIONS
        .iter()
        .copied()
        .filter(|item| item.version > report.source_version)
    {
        if migration.version == 27 {
            apply_notification_scope_repairs(conn, &report.findings).await?;
        }
        if migration.version == 14 {
            sqlx::query(
                "CREATE TEMP TABLE _audited_pre014_aliases AS SELECT u.id alias,oa.provider_id user_id FROM users u JOIN oauth_accounts oa ON oa.user_id=u.id AND oa.provider='atproto' WHERE oa.provider_id IS NOT NULL AND oa.provider_id<>u.id",
            )
            .execute(&mut *conn)
            .await?;
        }
        sqlx::raw_sql(migration.sql).execute(&mut *conn).await?;
        sqlx::query("INSERT OR IGNORE INTO schema_version(version) VALUES(?)")
            .bind(migration.version)
            .execute(&mut *conn)
            .await?;
    }
    if report.source_version < 14 && object_exists(conn, "table", "user_aliases").await? {
        if object_exists(conn, "table", "channel_permission_overrides").await? {
            sqlx::query(
                "INSERT INTO migration_repair_log( \
                    migration_version,repair_kind,object_type,object_id,outcome,details \
                 ) SELECT 17,'pre014_user_override','channel_permission_override',o.id, \
                          'repaired',json_object( \
                              'previous_target_id',o.target_id, \
                              'target_user_id',a.user_id, \
                              'allow_bits',o.allow_bits, \
                              'deny_bits',o.deny_bits, \
                              'evidence','unique AT Protocol subject mapping' \
                          ) \
                   FROM channel_permission_overrides o \
                   JOIN _audited_pre014_aliases a ON a.alias=o.target_id \
                  WHERE o.target_type='user' \
                    AND EXISTS(SELECT 1 FROM users u WHERE u.id=a.user_id)",
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "UPDATE channel_permission_overrides AS o \
                    SET target_id=(SELECT a.user_id FROM _audited_pre014_aliases a \
                                   WHERE a.alias=o.target_id) \
                  WHERE o.target_type='user' \
                    AND EXISTS(SELECT 1 FROM _audited_pre014_aliases a \
                               JOIN users u ON u.id=a.user_id WHERE a.alias=o.target_id)",
            )
            .execute(&mut *conn)
            .await?;
        }
        sqlx::query("INSERT OR IGNORE INTO user_aliases(alias,user_id,alias_kind) SELECT alias,user_id,'legacy_id' FROM _audited_pre014_aliases WHERE EXISTS(SELECT 1 FROM users WHERE users.id=_audited_pre014_aliases.user_id)")
            .execute(&mut *conn).await?;
        sqlx::query("DROP TABLE _audited_pre014_aliases")
            .execute(&mut *conn)
            .await?;
    }
    if report.source_version < 17 {
        for migration in MIGRATIONS.iter().filter(|item| item.version < 17) {
            let provenance = if migration.version <= report.source_version {
                "adopted_release_effects"
            } else {
                "bundled_script"
            };
            sqlx::query(
                "INSERT INTO migration_metadata(version,checksum_sha256,provenance) VALUES (?,?,?)",
            )
            .bind(migration.version)
            .bind(checksum(migration.sql))
            .bind(provenance)
            .execute(&mut *conn)
            .await?;
        }
        let migration = MIGRATIONS
            .iter()
            .find(|item| item.version == 17)
            .expect("migration 17 exists");
        sqlx::query("INSERT INTO migration_metadata(version,checksum_sha256,provenance) VALUES (?,?,'bundled_script')")
            .bind(migration.version).bind(checksum(migration.sql)).execute(&mut *conn).await?;
        sqlx::query("INSERT INTO database_metadata(singleton,compatibility_floor,generation) VALUES (1,?,?)")
            .bind(COMPATIBILITY_FLOOR).bind(Uuid::new_v4().to_string()).execute(&mut *conn).await?;
    }
    apply_safe_repairs(conn, &report.findings).await?;
    if report.source_version < 2 {
        let generated = inspect_repairs(conn, LATEST_SCHEMA_VERSION).await?;
        apply_safe_repairs(conn, &generated).await?;
        let generated = inspect_repairs(conn, LATEST_SCHEMA_VERSION).await?;
        if generated.iter().any(|item| item.blocks_upgrade) {
            transaction.rollback().await?;
            return Err(MigrationError::Preflight(MigrationPreflightReport {
                source_version: report.source_version,
                target_version: LATEST_SCHEMA_VERSION,
                findings: generated,
            }));
        }
    }
    record_snapshot(conn, &snapshot).await?;
    if object_exists(conn, "table", "messages_fts").await? {
        sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")
            .execute(&mut *conn)
            .await?;
    }
    for migration in MIGRATIONS
        .iter()
        .filter(|item| item.version > report.source_version && item.version > 17)
    {
        sqlx::query("INSERT INTO migration_metadata(version,checksum_sha256,provenance) VALUES (?,?,'bundled_script')")
            .bind(migration.version)
            .bind(checksum(migration.sql))
            .execute(&mut *conn)
            .await?;
    }
    verify_integrity(conn).await?;
    transaction.commit().await?;

    let mut checked = pool.acquire().await?;
    verify_integrity(&mut checked).await?;
    let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *checked)
        .await?;
    if enabled != 1 {
        return Err(MigrationError::Integrity {
            check: "connection profile",
            detail: "foreign_keys was not restored".into(),
        });
    }
    info!(
        version = LATEST_SCHEMA_VERSION,
        "database migrations verified"
    );
    metric.succeed();
    Ok(())
}
