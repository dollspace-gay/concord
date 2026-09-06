use super::{
    Digest, LATEST_SCHEMA_VERSION, MIGRATIONS, MigrationError, Row, Sha256, SqliteConnection,
};
use sqlx::Connection;

pub(super) fn checksum(sql: &str) -> String {
    hex::encode(Sha256::digest(sql.as_bytes()))
}

pub(super) async fn object_exists(
    conn: &mut SqliteConnection,
    kind: &str,
    name: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ? AND name = ?)")
        .bind(kind)
        .bind(name)
        .fetch_one(conn)
        .await
}

pub(super) async fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    let quoted = table.replace('"', "\"\"");
    // The identifier is double-quoted with embedded quotes escaped above.
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA table_info(\"{quoted}\")"
    )))
    .fetch_all(conn)
    .await?;
    Ok(rows
        .iter()
        .any(|row| row.get::<String, _>("name") == column))
}

pub(super) async fn require_effect(
    conn: &mut SqliteConnection,
    version: i64,
) -> Result<Option<String>, sqlx::Error> {
    let (kind, name, extra) = match version {
        1 => ("table", "users", None),
        2 => ("table", "servers", Some(("channels", "server_id"))),
        3 => ("table", "reactions", Some(("messages", "edited_at"))),
        4 => ("table", "attachments", None),
        5 => ("column", "blob_cid", Some(("attachments", "blob_cid"))),
        6 => ("table", "server_config", None),
        7 => (
            "table",
            "channel_permission_overrides",
            Some(("channels", "is_private")),
        ),
        8 => (
            "table",
            "notification_settings",
            Some(("messages", "deleted_at")),
        ),
        9 => (
            "table",
            "pinned_messages",
            Some(("channels", "channel_type")),
        ),
        10 => ("table", "audit_log", Some(("channels", "is_nsfw"))),
        11 => ("table", "invites", Some(("servers", "description"))),
        12 => ("table", "webhooks", Some(("users", "is_bot"))),
        13 => (
            "table",
            "bsky_shared_posts",
            Some(("oauth_accounts", "bsky_handle")),
        ),
        14 => ("table", "users", None),
        15 => ("table", "stickers", Some(("servers", "vanity_code"))),
        16 => ("trigger", "messages_fts_hard_delete", None),
        17 => (
            "table",
            "migration_metadata",
            Some(("database_metadata", "generation")),
        ),
        18 => ("table", "auth_credentials", Some(("users", "disabled_at"))),
        19 => (
            "table",
            "thread_members",
            Some(("channels", "parent_channel_id")),
        ),
        20 => (
            "table",
            "conversations",
            Some(("messages", "conversation_id")),
        ),
        21 => (
            "table",
            "command_receipts",
            Some(("entity_versions", "version")),
        ),
        22 => (
            "table",
            "user_aliases",
            Some(("conversation_participants", "user_id")),
        ),
        23 => (
            "table",
            "media_import_ledger",
            Some(("attachments", "media_state")),
        ),
        _ => return Ok(Some(format!("unrecognized migration version {version}"))),
    };
    let present = if kind == "column" {
        let (table, column) = extra.expect("column fingerprint has its table");
        column_exists(conn, table, column).await?
    } else {
        object_exists(conn, kind, name).await?
    };
    if !present {
        return Ok(Some(format!(
            "version {version} effect {kind} {name} is missing"
        )));
    }
    if kind != "column"
        && let Some((table, column)) = extra
        && !column_exists(conn, table, column).await?
    {
        return Ok(Some(format!(
            "version {version} effect {table}.{column} is missing"
        )));
    }
    Ok(None)
}

pub(super) async fn source_version(conn: &mut SqliteConnection) -> Result<i64, MigrationError> {
    if !object_exists(conn, "table", "schema_version").await? {
        if !object_exists(conn, "table", "users").await? {
            return Ok(0);
        }
        if require_effect(conn, 1).await?.is_none()
            && !object_exists(conn, "table", "servers").await?
        {
            return Ok(1);
        }
        return Err(MigrationError::Integrity {
            check: "schema recognition",
            detail: "application tables exist without recognized schema_version history".into(),
        });
    }
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_version ORDER BY version")
            .fetch_all(&mut *conn)
            .await?;
    let Some(&last) = versions.last() else {
        return Err(MigrationError::Integrity {
            check: "history",
            detail: "schema_version is empty".into(),
        });
    };
    if last > LATEST_SCHEMA_VERSION {
        return Err(MigrationError::Integrity {
            check: "history",
            detail: format!(
                "database version {last} exceeds recognized version {LATEST_SCHEMA_VERSION}"
            ),
        });
    }
    let from_one: Vec<i64> = (1..=last).collect();
    let from_two: Vec<i64> = (2..=last).collect();
    if versions != from_one && versions != from_two {
        return Err(MigrationError::Integrity {
            check: "history",
            detail: format!("versions are not contiguous: {versions:?}"),
        });
    }
    Ok(last)
}

pub(super) fn normalize_schema_sql(sql: Option<String>) -> String {
    let mut normalized = String::new();
    let mut quote = None;
    let source = sql.unwrap_or_default();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match quote {
            Some(delimiter) => {
                normalized.push(character);
                if character == delimiter {
                    if characters.peek() == Some(&delimiter) {
                        normalized.push(characters.next().expect("peeked quote exists"));
                    } else {
                        quote = None;
                    }
                }
            }
            None if matches!(character, '\'' | '"') => {
                quote = Some(character);
                normalized.push(character);
            }
            None if character.is_whitespace() => {}
            None => normalized.extend(character.to_lowercase()),
        }
    }
    normalized
}

pub(super) async fn schema_fingerprint(
    conn: &mut SqliteConnection,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            format!(
                "{}|{}|{}|{}",
                row.get::<String, _>(0),
                row.get::<String, _>(1),
                row.get::<String, _>(2),
                normalize_schema_sql(row.get::<Option<String>, _>(3))
            )
        })
        .collect())
}

pub(super) async fn expected_fingerprint(
    version: i64,
    has_ledger: bool,
) -> Result<Vec<String>, sqlx::Error> {
    let mut conn = SqliteConnection::connect("sqlite::memory:").await?;
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut conn)
        .await?;
    if has_ledger {
        sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))")
            .execute(&mut conn).await?;
    }
    for migration in MIGRATIONS.iter().filter(|item| item.version <= version) {
        sqlx::raw_sql(migration.sql).execute(&mut conn).await?;
        if has_ledger {
            sqlx::query("INSERT OR IGNORE INTO schema_version(version) VALUES(?)")
                .bind(migration.version)
                .execute(&mut conn)
                .await?;
        }
    }
    schema_fingerprint(&mut conn).await
}
