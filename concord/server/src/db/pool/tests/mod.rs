use super::introspection::normalize_schema_sql;
use super::*;

async fn historical_fixture(version: i64) -> SqlitePool {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *conn)
        .await
        .unwrap();
    if version >= 1 {
        sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))")
            .execute(&mut *conn).await.unwrap();
    }
    for migration in MIGRATIONS.iter().filter(|item| item.version <= version) {
        sqlx::raw_sql(migration.sql)
            .execute(&mut *conn)
            .await
            .unwrap();
        if version >= 1 {
            sqlx::query("INSERT OR IGNORE INTO schema_version(version) VALUES(?)")
                .bind(migration.version)
                .execute(&mut *conn)
                .await
                .unwrap();
        }
    }
    if version >= 17 {
        for migration in MIGRATIONS.iter().filter(|item| item.version <= version) {
            sqlx::query(
                "INSERT INTO migration_metadata(version,checksum_sha256,provenance) \
                 VALUES(?,?,'adopted_release_effects')",
            )
            .bind(migration.version)
            .bind(checksum(migration.sql))
            .execute(&mut *conn)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO database_metadata(singleton,compatibility_floor,generation) \
             VALUES(1,?,'historical-fixture')",
        )
        .bind(COMPATIBILITY_FLOOR)
        .execute(&mut *conn)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
        .bind(format!("legacy-user-{version}"))
        .bind(format!("legacy{version}"))
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);
    pool
}

mod authorization;
mod behavior;
mod identity;
mod recovery;
mod validation;
