use super::{
    LATEST_SCHEMA_VERSION, MIGRATIONS, MigrationError, MigrationPreflightReport, SqliteConnection,
    checksum, expected_fingerprint, inspect_repairs, schema_fingerprint, source_version,
};

pub(super) async fn migration_preflight_connection(
    conn: &mut SqliteConnection,
) -> Result<MigrationPreflightReport, MigrationError> {
    let version = source_version(&mut *conn).await?;
    let actual = schema_fingerprint(&mut *conn).await?;
    let expected = expected_fingerprint(version, version >= 1).await?;
    let unledgered_v1 = if version == 1 {
        Some(expected_fingerprint(version, false).await?)
    } else {
        None
    };
    if actual != expected && unledgered_v1.as_ref() != Some(&actual) {
        let first_difference = actual
            .iter()
            .zip(expected.iter())
            .position(|(left, right)| left != right)
            .unwrap_or(actual.len().min(expected.len()));
        return Err(MigrationError::Integrity {
            check: "schema fingerprint",
            detail: format!(
                "unrecognized schema drift at entry {first_difference}; actual objects={}, expected objects={}",
                actual.len(),
                expected.len()
            ),
        });
    }
    if version >= 17 {
        for migration in MIGRATIONS.iter().filter(|item| item.version <= version) {
            let stored: Option<String> = sqlx::query_scalar(
                "SELECT checksum_sha256 FROM migration_metadata WHERE version=?",
            )
            .bind(migration.version)
            .fetch_optional(&mut *conn)
            .await?;
            match stored {
                Some(stored) if stored == checksum(migration.sql) => {}
                Some(stored) => {
                    return Err(MigrationError::Integrity {
                        check: "checksum",
                        detail: format!(
                            "{} expected {}, stored {stored}",
                            migration.name,
                            checksum(migration.sql)
                        ),
                    });
                }
                None => {
                    return Err(MigrationError::Integrity {
                        check: "metadata",
                        detail: format!("{} has no checksum", migration.name),
                    });
                }
            }
        }
    }
    Ok(MigrationPreflightReport {
        source_version: version,
        target_version: LATEST_SCHEMA_VERSION,
        findings: inspect_repairs(&mut *conn, version).await?,
    })
}

pub(super) async fn capture_snapshot(
    conn: &mut SqliteConnection,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts%' ORDER BY name").fetch_all(&mut *conn).await?;
    let mut snapshot = Vec::with_capacity(tables.len());
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        // SQLite schema names are double-quoted with embedded quotes escaped above.
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM \"{quoted}\""
        )))
        .fetch_one(&mut *conn)
        .await?;
        snapshot.push((table, count));
    }
    Ok(snapshot)
}

pub(super) async fn record_snapshot(
    conn: &mut SqliteConnection,
    snapshot: &[(String, i64)],
) -> Result<(), sqlx::Error> {
    for (table, count) in snapshot {
        sqlx::query("INSERT INTO migration_snapshots(migration_version,table_name,row_count) VALUES (?,?,?)")
            .bind(LATEST_SCHEMA_VERSION).bind(table).bind(count).execute(&mut *conn).await?;
    }
    Ok(())
}
