use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use tracing::info;

/// Create and initialize a SQLite connection pool with WAL mode.
pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    info!("database connected: {}", database_url);
    Ok(pool)
}

/// Run all pending migration SQL files against the database.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Ensure schema_version table exists for tracking
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version     INTEGER PRIMARY KEY, \
            applied_at  TEXT NOT NULL DEFAULT (datetime('now'))\
        )",
    )
    .execute(pool)
    .await?;

    let current_version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_version")
            .fetch_one(pool)
            .await?;

    let migrations: &[(i64, &str)] = &[
        (1, include_str!("../../migrations/001_initial.sql")),
        (2, include_str!("../../migrations/002_servers.sql")),
    ];

    for &(version, sql) in migrations {
        if version <= current_version {
            continue;
        }
        info!("applying migration {version}...");
        // Use a single connection so PRAGMAs persist across statements
        let mut conn = pool.acquire().await?;
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await?;
        for statement in sql.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(&mut *conn).await?;
            }
        }
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await?;
        // Migration 1 doesn't record its own version, so do it here
        if version == 1 {
            sqlx::query("INSERT OR IGNORE INTO schema_version (version) VALUES (?)")
                .bind(version)
                .execute(&mut *conn)
                .await?;
        }
    }

    let final_version = migrations.last().map(|m| m.0).unwrap_or(0);
    info!("database migrations applied (version: {final_version})");
    Ok(())
}
