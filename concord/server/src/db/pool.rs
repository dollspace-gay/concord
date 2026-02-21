use sqlx::Connection;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;
use tracing::info;

/// Create and initialize a SQLite connection pool with WAL mode.
pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .create_if_missing(true)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    info!("database connected: {}", database_url);
    Ok(pool)
}

/// Split SQL text into statements, respecting BEGIN...END blocks (triggers).
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_begin = false;

    for line in sql.lines() {
        let trimmed = line.trim();
        // Skip pure comment lines outside of a statement
        if trimmed.starts_with("--") && current.trim().is_empty() {
            continue;
        }

        current.push_str(line);
        current.push('\n');

        let upper = trimmed.to_uppercase();
        if upper.starts_with("BEGIN") || upper.ends_with(" BEGIN") {
            in_begin = true;
        }

        if in_begin {
            if upper.starts_with("END;") || upper == "END" {
                in_begin = false;
                let stmt = current.trim().to_string();
                // Remove trailing semicolon for consistency
                let stmt = stmt.strip_suffix(';').unwrap_or(&stmt).trim().to_string();
                if !stmt.is_empty() {
                    statements.push(stmt);
                }
                current.clear();
            }
        } else {
            // Outside BEGIN..END: split on semicolons
            while let Some(pos) = current.find(';') {
                let stmt = current[..pos].trim().to_string();
                if !stmt.is_empty() && !stmt.starts_with("--") {
                    statements.push(stmt);
                }
                current = current[pos + 1..].to_string();
            }
        }
    }

    // Any remaining text
    let remaining = current.trim().to_string();
    if !remaining.is_empty() && !remaining.starts_with("--") {
        let remaining = remaining
            .strip_suffix(';')
            .unwrap_or(&remaining)
            .trim()
            .to_string();
        if !remaining.is_empty() {
            statements.push(remaining);
        }
    }

    statements
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
        (
            3,
            include_str!("../../migrations/003_messaging_enhancements.sql"),
        ),
        (4, include_str!("../../migrations/004_media_files.sql")),
        (
            5,
            include_str!("../../migrations/005_atproto_blob_storage.sql"),
        ),
        (6, include_str!("../../migrations/006_server_config.sql")),
        (
            7,
            include_str!("../../migrations/007_organization_permissions.sql"),
        ),
        (8, include_str!("../../migrations/008_user_experience.sql")),
        (9, include_str!("../../migrations/009_threads_pinning.sql")),
        (10, include_str!("../../migrations/010_moderation.sql")),
        (11, include_str!("../../migrations/011_community.sql")),
        (12, include_str!("../../migrations/012_integrations.sql")),
        (
            13,
            include_str!("../../migrations/013_atproto_integration.sql"),
        ),
        (14, include_str!("../../migrations/014_user_id_to_did.sql")),
        (
            15,
            include_str!("../../migrations/015_premium_for_free.sql"),
        ),
        (
            16,
            include_str!("../../migrations/016_fts_delete_trigger.sql"),
        ),
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
        // Run the migration in an inner block so we can always re-enable
        // foreign keys even if the migration fails.
        let result: Result<(), sqlx::Error> = async {
            // Wrap all migration statements + version recording in a transaction
            // so a partial failure cannot leave the schema in an inconsistent state.
            let mut tx = conn.begin().await?;
            for statement in split_sql_statements(sql) {
                if !statement.is_empty() {
                    sqlx::query(&statement).execute(&mut *tx).await?;
                }
            }
            // Record the migration version inside the same transaction
            sqlx::query("INSERT OR IGNORE INTO schema_version (version) VALUES (?)")
                .bind(version)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        // Always re-enable foreign keys, even if the migration failed
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await?;
        // Now propagate any migration error
        result?;
    }

    // Rebuild FTS index to fix any duplicates from prior migration re-runs
    let fts_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='messages_fts'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    if fts_exists {
        let mut conn = pool.acquire().await?;
        let _ = sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")
            .execute(&mut *conn)
            .await;
    }

    let final_version = migrations.last().map(|m| m.0).unwrap_or(0);
    info!("database migrations applied (version: {final_version})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── split_sql_statements unit tests ─────────────────────────

    #[test]
    fn test_split_simple_statements() {
        let sql = "CREATE TABLE a (id INT);\nCREATE TABLE b (id INT);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "CREATE TABLE a (id INT)");
        assert_eq!(stmts[1], "CREATE TABLE b (id INT)");
    }

    #[test]
    fn test_split_skips_comment_lines() {
        let sql = "-- This is a comment\nCREATE TABLE a (id INT);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
        assert_eq!(stmts[0], "CREATE TABLE a (id INT)");
    }

    #[test]
    fn test_split_handles_begin_end_blocks() {
        let sql = "\
CREATE TABLE a (id INT);
CREATE TRIGGER trg AFTER INSERT ON a BEGIN
  UPDATE a SET id = id + 1;
END;
CREATE TABLE b (id INT);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 3);
        assert_eq!(stmts[0], "CREATE TABLE a (id INT)");
        assert!(
            stmts[1].contains("CREATE TRIGGER"),
            "Trigger statement should be kept intact"
        );
        assert!(
            stmts[1].contains("UPDATE a SET id = id + 1;"),
            "Inner semicolons should not split the trigger"
        );
        assert_eq!(stmts[2], "CREATE TABLE b (id INT)");
    }

    #[test]
    fn test_split_empty_input() {
        let stmts = split_sql_statements("");
        assert!(stmts.is_empty());
    }

    #[test]
    fn test_split_only_comments() {
        let sql = "-- comment 1\n-- comment 2\n";
        let stmts = split_sql_statements(sql);
        assert!(stmts.is_empty());
    }

    #[test]
    fn test_split_multiple_triggers() {
        let sql = "\
CREATE TRIGGER trg1 AFTER INSERT ON a BEGIN
  UPDATE b SET x = 1;
END;
CREATE TRIGGER trg2 AFTER DELETE ON a BEGIN
  DELETE FROM b WHERE x = 1;
END;";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("trg1"));
        assert!(stmts[1].contains("trg2"));
    }

    #[test]
    fn test_split_trailing_whitespace() {
        let sql = "  CREATE TABLE a (id INT);  \n  ";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
        assert_eq!(stmts[0], "CREATE TABLE a (id INT)");
    }

    // ── Migration integration tests ─────────────────────────────

    #[tokio::test]
    async fn test_migration_version_recording_with_insert_or_ignore() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Check versions are recorded
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 16);

        // Running again should not duplicate (INSERT OR IGNORE)
        run_migrations(&pool).await.unwrap();

        let count_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count_after, 16, "No duplicate version rows after re-run");
    }

    #[tokio::test]
    async fn test_fts_index_rebuilt_on_startup() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Insert a test message so FTS has data
        sqlx::query("INSERT INTO users (id, username) VALUES ('u1', 'alice')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers (id, name, owner_id) VALUES ('s1', 'Test', 'u1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels (id, server_id, name) VALUES ('c1', 's1', '#gen')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, server_id, channel_id, sender_id, sender_nick, content) \
             VALUES ('m1', 's1', 'c1', 'u1', 'alice', 'searchable text here')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Run migrations again (triggers FTS rebuild)
        run_migrations(&pool).await.unwrap();

        // FTS should be queryable
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'searchable'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(found >= 1, "FTS index should contain the inserted message");
    }

    #[tokio::test]
    async fn test_each_migration_version_is_sequential() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM schema_version ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        let expected: Vec<i64> = (1..=16).collect();
        assert_eq!(
            versions, expected,
            "Migration versions should be 1 through 12"
        );
    }
}
