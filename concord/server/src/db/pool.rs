use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Connection, Row, SqliteConnection, SqlitePool};
use std::{fmt, str::FromStr, time::Duration};
use tracing::info;
use uuid::Uuid;

const LATEST_SCHEMA_VERSION: i64 = 32;
const COMPATIBILITY_FLOOR: i64 = 17;

pub const fn current_schema_version() -> i64 {
    LATEST_SCHEMA_VERSION
}

#[derive(Clone, Copy)]
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_initial.sql",
        sql: include_str!("../../migrations/001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "002_servers.sql",
        sql: include_str!("../../migrations/002_servers.sql"),
    },
    Migration {
        version: 3,
        name: "003_messaging_enhancements.sql",
        sql: include_str!("../../migrations/003_messaging_enhancements.sql"),
    },
    Migration {
        version: 4,
        name: "004_media_files.sql",
        sql: include_str!("../../migrations/004_media_files.sql"),
    },
    Migration {
        version: 5,
        name: "005_atproto_blob_storage.sql",
        sql: include_str!("../../migrations/005_atproto_blob_storage.sql"),
    },
    Migration {
        version: 6,
        name: "006_server_config.sql",
        sql: include_str!("../../migrations/006_server_config.sql"),
    },
    Migration {
        version: 7,
        name: "007_organization_permissions.sql",
        sql: include_str!("../../migrations/007_organization_permissions.sql"),
    },
    Migration {
        version: 8,
        name: "008_user_experience.sql",
        sql: include_str!("../../migrations/008_user_experience.sql"),
    },
    Migration {
        version: 9,
        name: "009_threads_pinning.sql",
        sql: include_str!("../../migrations/009_threads_pinning.sql"),
    },
    Migration {
        version: 10,
        name: "010_moderation.sql",
        sql: include_str!("../../migrations/010_moderation.sql"),
    },
    Migration {
        version: 11,
        name: "011_community.sql",
        sql: include_str!("../../migrations/011_community.sql"),
    },
    Migration {
        version: 12,
        name: "012_integrations.sql",
        sql: include_str!("../../migrations/012_integrations.sql"),
    },
    Migration {
        version: 13,
        name: "013_atproto_integration.sql",
        sql: include_str!("../../migrations/013_atproto_integration.sql"),
    },
    Migration {
        version: 14,
        name: "014_user_id_to_did.sql",
        sql: include_str!("../../migrations/014_user_id_to_did.sql"),
    },
    Migration {
        version: 15,
        name: "015_premium_for_free.sql",
        sql: include_str!("../../migrations/015_premium_for_free.sql"),
    },
    Migration {
        version: 16,
        name: "016_fts_delete_trigger.sql",
        sql: include_str!("../../migrations/016_fts_delete_trigger.sql"),
    },
    Migration {
        version: 17,
        name: "017_migration_foundation.sql",
        sql: include_str!("../../migrations/017_migration_foundation.sql"),
    },
    Migration {
        version: 18,
        name: "018_session_authority.sql",
        sql: include_str!("../../migrations/018_session_authority.sql"),
    },
    Migration {
        version: 19,
        name: "019_authorization_threads.sql",
        sql: include_str!("../../migrations/019_authorization_threads.sql"),
    },
    Migration {
        version: 20,
        name: "020_conversations_messages.sql",
        sql: include_str!("../../migrations/020_conversations_messages.sql"),
    },
    Migration {
        version: 21,
        name: "021_receipts_events.sql",
        sql: include_str!("../../migrations/021_receipts_events.sql"),
    },
    Migration {
        version: 22,
        name: "022_identity_direct_presence.sql",
        sql: include_str!("../../migrations/022_identity_direct_presence.sql"),
    },
    Migration {
        version: 23,
        name: "023_private_media.sql",
        sql: include_str!("../../migrations/023_private_media.sql"),
    },
    Migration {
        version: 24,
        name: "024_feature_integrity.sql",
        sql: include_str!("../../migrations/024_feature_integrity.sql"),
    },
    Migration {
        version: 25,
        name: "025_operation_generations.sql",
        sql: include_str!("../../migrations/025_operation_generations.sql"),
    },
    Migration {
        version: 26,
        name: "026_integration_contracts.sql",
        sql: include_str!("../../migrations/026_integration_contracts.sql"),
    },
    Migration {
        version: 27,
        name: "027_moderation_notification_integrity.sql",
        sql: include_str!("../../migrations/027_moderation_notification_integrity.sql"),
    },
    Migration {
        version: 28,
        name: "028_server_member_nicknames.sql",
        sql: include_str!("../../migrations/028_server_member_nicknames.sql"),
    },
    Migration {
        version: 29,
        name: "029_oauth2_lifecycle.sql",
        sql: include_str!("../../migrations/029_oauth2_lifecycle.sql"),
    },
    Migration {
        version: 30,
        name: "030_role_projection_versions.sql",
        sql: include_str!("../../migrations/030_role_projection_versions.sql"),
    },
    Migration {
        version: 31,
        name: "031_message_chronology_index.sql",
        sql: include_str!("../../migrations/031_message_chronology_index.sql"),
    },
    Migration {
        version: 32,
        name: "032_operator_audit.sql",
        sql: include_str!("../../migrations/032_operator_audit.sql"),
    },
];

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct RepairFinding {
    pub code: &'static str,
    pub object_type: &'static str,
    pub object_id: String,
    pub detail: String,
    pub blocks_upgrade: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct MigrationPreflightReport {
    pub source_version: i64,
    pub target_version: i64,
    pub findings: Vec<RepairFinding>,
}

impl MigrationPreflightReport {
    pub fn is_blocked(&self) -> bool {
        self.findings.iter().any(|item| item.blocks_upgrade)
    }
}

#[derive(Debug)]
pub enum MigrationError {
    Database(sqlx::Error),
    Preflight(MigrationPreflightReport),
    Integrity { check: &'static str, detail: String },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(f, "migration database operation failed: {error}"),
            Self::Preflight(report) => {
                write!(
                    f,
                    "migration preflight rejected schema version {}",
                    report.source_version
                )?;
                for item in &report.findings {
                    write!(
                        f,
                        "; {} {} {}: {}",
                        item.code, item.object_type, item.object_id, item.detail
                    )?;
                }
                Ok(())
            }
            Self::Integrity { check, detail } => write!(f, "migration {check} failed: {detail}"),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for MigrationError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

/// Create a pool whose every connection uses the declared durable profile.
pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .after_connect(|connection, _| Box::pin(async move {
            sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *connection).await?;
            sqlx::query("PRAGMA synchronous = FULL").execute(&mut *connection).await?;
            let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys").fetch_one(&mut *connection).await?;
            let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous").fetch_one(&mut *connection).await?;
            if foreign_keys != 1 || synchronous != 2 {
                return Err(sqlx::Error::Protocol(format!("SQLite profile rejected: foreign_keys={foreign_keys}, synchronous={synchronous}")));
            }
            Ok(())
        }))
        .connect_with(options).await?;
    info!(
        database_url,
        "database connected with WAL, synchronous=FULL, foreign_keys=ON"
    );
    Ok(pool)
}

#[cfg(feature = "storage-fault-injection")]
unsafe extern "C" {
    fn concord_storage_fault_vfs_install() -> std::ffi::c_int;
    fn concord_storage_fault_vfs_arm_next_sync();
    fn concord_storage_fault_vfs_observed() -> std::ffi::c_int;
}

#[cfg(feature = "storage-fault-injection")]
static STORAGE_FAULT_VFS_INSTALL: std::sync::OnceLock<Result<(), std::ffi::c_int>> =
    std::sync::OnceLock::new();

/// Create the real production SQLx pool after installing the test-only VFS as
/// SQLite's process-wide default. The caller must use an isolated probe process
/// and invoke this before any SQLite connection is opened.
#[cfg(feature = "storage-fault-injection")]
pub async fn create_storage_fault_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let installed = STORAGE_FAULT_VFS_INSTALL.get_or_init(|| {
        // SAFETY: the C entry point takes no pointers. OnceLock serializes the
        // process-global registration and prevents concurrent safe callers
        // from racing the C VFS state.
        let result = unsafe { concord_storage_fault_vfs_install() };
        if result == libsqlite3_sys::SQLITE_OK {
            Ok(())
        } else {
            Err(result)
        }
    });
    if let Err(result) = installed {
        return Err(sqlx::Error::Protocol(format!(
            "failed to install Concord storage-fault VFS: SQLite result {result}"
        )));
    }
    create_pool(database_url).await
}

/// Fail the next synchronization of the isolated probe database.
#[cfg(feature = "storage-fault-injection")]
pub fn arm_storage_sync_fault() {
    // SAFETY: the entry point has no pointer or lifetime requirements. The
    // probe calls it only after `create_storage_fault_pool` succeeds.
    unsafe { concord_storage_fault_vfs_arm_next_sync() };
}

/// Return the number of injected xSync failures observed by the probe VFS.
#[cfg(feature = "storage-fault-injection")]
#[must_use]
pub fn observed_storage_sync_faults() -> u32 {
    // SAFETY: the entry point only performs an atomic load and returns by value.
    unsafe { concord_storage_fault_vfs_observed() as u32 }
}

fn checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.as_bytes()))
}

async fn object_exists(
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

async fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    let quoted = table.replace('"', "\"\"");
    let rows = sqlx::query(&format!("PRAGMA table_info(\"{quoted}\")"))
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .any(|row| row.get::<String, _>("name") == column))
}

async fn require_effect(
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

async fn source_version(conn: &mut SqliteConnection) -> Result<i64, MigrationError> {
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

fn normalize_schema_sql(sql: Option<String>) -> String {
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

async fn schema_fingerprint(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
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

async fn expected_fingerprint(version: i64, has_ledger: bool) -> Result<Vec<String>, sqlx::Error> {
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

async fn inspect_repairs(
    conn: &mut SqliteConnection,
    version: i64,
) -> Result<Vec<RepairFinding>, sqlx::Error> {
    let mut findings = Vec::new();
    if version >= 1 {
        let query = if version == 1 {
            "SELECT c.name, c.name, EXISTS(SELECT 1 FROM messages m WHERE m.channel_name=c.name), EXISTS(SELECT 1 FROM channel_members cm WHERE cm.channel_name=c.name) FROM channels c WHERE c.name IN ('#general','#random') AND c.is_default=1"
        } else {
            "SELECT c.id, c.name, EXISTS(SELECT 1 FROM messages m WHERE m.channel_id=c.id), EXISTS(SELECT 1 FROM channel_members cm WHERE cm.channel_id=c.id) FROM channels c WHERE c.server_id='default' AND NOT EXISTS(SELECT 1 FROM servers s WHERE s.id=c.server_id)"
        };
        for row in sqlx::query(query).fetch_all(&mut *conn).await? {
            let id: String = row.get(0);
            let name: String = row.get(1);
            let populated = row.get::<i64, _>(2) != 0 || row.get::<i64, _>(3) != 0;
            let known = matches!(name.as_str(), "#general" | "#random");
            findings.push(RepairFinding {
                code: "legacy_default_server",
                object_type: "channel",
                object_id: id,
                detail: if !populated && known {
                    "known generated empty channel will be removed and recorded".into()
                } else {
                    "orphan channel is populated or unrecognized; operator server mapping required"
                        .into()
                },
                blocks_upgrade: populated || !known,
            });
        }
    }
    if version >= 14 && object_exists(conn, "table", "channel_permission_overrides").await? {
        let rows = sqlx::query("SELECT id,target_id FROM channel_permission_overrides o WHERE target_type='user' AND NOT EXISTS(SELECT 1 FROM users u WHERE u.id=o.target_id)").fetch_all(&mut *conn).await?;
        for row in rows {
            findings.push(RepairFinding { code: "unresolved_user_override", object_type: "channel_permission_override", object_id: row.get(0), detail: format!("target {} is not a current user; audited identity mapping required and grant remains denied", row.get::<String, _>(1)), blocks_upgrade: true });
        }
    }
    if (7..14).contains(&version)
        && object_exists(conn, "table", "oauth_accounts").await?
        && object_exists(conn, "table", "channel_permission_overrides").await?
    {
        let rows = sqlx::query(
            "SELECT user_id,group_concat(provider_id) FROM oauth_accounts \
             WHERE provider='atproto' AND provider_id IS NOT NULL AND provider_id<>user_id \
             GROUP BY user_id HAVING count(DISTINCT provider_id)>1",
        )
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            findings.push(RepairFinding {
                code: "ambiguous_pre014_at_identity",
                object_type: "user",
                object_id: row.get(0),
                detail: format!(
                    "multiple AT Protocol subjects ({}) claim this legacy user; no identity or permission mapping can be inferred",
                    row.get::<String, _>(1)
                ),
                blocks_upgrade: true,
            });
        }
        let rows = sqlx::query(
            "SELECT legacy.id,legacy.target_id,account.provider_id,current.id \
             FROM channel_permission_overrides legacy \
             JOIN oauth_accounts account \
               ON account.user_id=legacy.target_id AND account.provider='atproto' \
              AND account.provider_id IS NOT NULL AND account.provider_id<>account.user_id \
             JOIN channel_permission_overrides current \
               ON current.channel_id=legacy.channel_id AND current.target_type='user' \
              AND current.target_id=account.provider_id AND current.id<>legacy.id \
             WHERE legacy.target_type='user'",
        )
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            findings.push(RepairFinding {
                code: "pre014_override_target_collision",
                object_type: "channel_permission_override",
                object_id: row.get(0),
                detail: format!(
                    "legacy target {} maps to {}, already granted by override {}; operator must reconcile permission bits explicitly",
                    row.get::<String, _>(1),
                    row.get::<String, _>(2),
                    row.get::<String, _>(3)
                ),
                blocks_upgrade: true,
            });
        }
    }
    if version >= 9 && object_exists(conn, "table", "channels").await? {
        let has_parent_channel = column_exists(conn, "channels", "parent_channel_id").await?;
        let rows = if has_parent_channel {
            sqlx::query(
                "SELECT id,parent_channel_id FROM channels c \
                 WHERE c.channel_type IN ('thread','public_thread','private_thread') \
                   AND (c.parent_channel_id IS NULL OR NOT EXISTS( \
                       SELECT 1 FROM channels parent WHERE parent.id=c.parent_channel_id \
                   ))",
            )
            .fetch_all(&mut *conn)
            .await?
        } else {
            sqlx::query(
                "SELECT c.id,c.thread_parent_message_id FROM channels c \
                 WHERE c.channel_type IN ('thread','public_thread','private_thread') \
                   AND (c.thread_parent_message_id IS NULL OR NOT EXISTS( \
                       SELECT 1 FROM messages parent \
                       WHERE parent.id=c.thread_parent_message_id \
                         AND parent.channel_id IS NOT NULL \
                   ))",
            )
            .fetch_all(&mut *conn)
            .await?
        };
        for row in rows {
            findings.push(RepairFinding {
                code: "thread_parent_missing",
                object_type: "channel",
                object_id: row.get(0),
                detail: format!(
                    "thread references unavailable parent {:?}; operator must map a validated parent or quarantine the thread from visibility; destructive removal is not automatic",
                    row.get::<Option<String>, _>(1)
                ),
                blocks_upgrade: true,
            });
        }
    }
    if version >= 2 {
        for (table, column) in [
            ("messages", "created_at"),
            ("channels", "created_at"),
            ("servers", "created_at"),
            ("notification_settings", "updated_at"),
        ] {
            if !column_exists(conn, table, column).await? {
                continue;
            }
            let quoted_table = table.replace('"', "\"\"");
            let quoted_column = column.replace('"', "\"\"");
            let query = format!(
                "SELECT id,{quoted_column} FROM \"{quoted_table}\" \
                 WHERE {quoted_column} IS NULL OR julianday({quoted_column}) IS NULL"
            );
            for row in sqlx::query(&query).fetch_all(&mut *conn).await? {
                findings.push(RepairFinding {
                    code: "malformed_timestamp",
                    object_type: table,
                    object_id: row.get(0),
                    detail: format!(
                        "{column} value {:?} is not a recognized SQLite timestamp; explicit replacement required",
                        row.get::<Option<String>, _>(1)
                    ),
                    blocks_upgrade: true,
                });
            }
        }
        for table in ["users", "servers", "channels", "messages"] {
            if !object_exists(conn, "table", table).await? {
                continue;
            }
            let quoted = table.replace('"', "\"\"");
            let query = format!("SELECT id FROM \"{quoted}\" WHERE trim(id)='' OR length(id)>512");
            for row in sqlx::query(&query).fetch_all(&mut *conn).await? {
                findings.push(RepairFinding {
                    code: "malformed_identifier",
                    object_type: table,
                    object_id: row.get(0),
                    detail: "identifier is empty or exceeds the 512-byte repair bound; explicit remapping required".into(),
                    blocks_upgrade: true,
                });
            }
        }
    }
    if version >= 8 && object_exists(conn, "table", "notification_settings").await? {
        for row in sqlx::query(
            "SELECT id,mute_until FROM notification_settings \
             WHERE mute_until IS NOT NULL AND julianday(mute_until) IS NULL",
        )
        .fetch_all(&mut *conn)
        .await?
        {
            findings.push(RepairFinding {
                code: "malformed_timestamp",
                object_type: "notification_settings",
                object_id: row.get(0),
                detail: format!(
                    "mute_until value {:?} is not a recognized SQLite timestamp; explicit replacement required",
                    row.get::<Option<String>, _>(1)
                ),
                blocks_upgrade: true,
            });
        }
        for row in sqlx::query(
            "SELECT n.id,n.server_id,n.channel_id FROM notification_settings n \
             WHERE (n.channel_id IS NOT NULL AND n.server_id IS NULL) \
                OR (n.channel_id IS NOT NULL AND NOT EXISTS( \
                    SELECT 1 FROM channels c \
                    WHERE c.id=n.channel_id AND c.server_id=n.server_id \
                ))",
        )
        .fetch_all(&mut *conn)
        .await?
        {
            findings.push(RepairFinding {
                code: "malformed_notification_scope",
                object_type: "notification_settings",
                object_id: row.get(0),
                detail: format!(
                    "notification server={:?} channel={:?} is not global, server-scoped, or bound to a channel in that server",
                    row.get::<Option<String>, _>(1),
                    row.get::<Option<String>, _>(2)
                ),
                blocks_upgrade: true,
            });
        }
        let rows = sqlx::query(
            "SELECT user_id,server_id,channel_id,group_concat(id), \
                    count(DISTINCT level || ':' || suppress_everyone || ':' || \
                          suppress_roles || ':' || muted || ':' || COALESCE(mute_until,'')), \
                    sum(CASE WHEN julianday(updated_at) IS NULL THEN 1 ELSE 0 END) \
             FROM notification_settings \
             GROUP BY user_id,server_id,channel_id HAVING count(*)>1",
        )
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            let variants: i64 = row.get(4);
            let invalid_timestamps: i64 = row.get(5);
            findings.push(RepairFinding {
                code: "duplicate_notification_scope",
                object_type: "notification_settings",
                object_id: row.get(3),
                detail: format!(
                    "duplicate scope user={} server={:?} channel={:?} has {variants} distinct setting variant(s) and {invalid_timestamps} invalid updated_at value(s); valid rows are exported and the latest updated_at then greatest stable ID wins",
                    row.get::<String, _>(0),
                    row.get::<Option<String>, _>(1),
                    row.get::<Option<String>, _>(2)
                ),
                blocks_upgrade: invalid_timestamps != 0,
            });
        }
    }
    if version >= 2 {
        for row in sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *conn)
            .await?
        {
            let table: String = row.get(0);
            let row_id: i64 = row.get(1);
            let parent: String = row.get(2);
            let has_safe_default = findings
                .iter()
                .any(|item| item.code == "legacy_default_server" && !item.blocks_upgrade);
            let known_default = if !has_safe_default || parent != "servers" {
                false
            } else if table == "channels" {
                true
            } else if table == "channel_aliases" {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM channel_aliases a JOIN channels c \
                     ON c.id=a.channel_id WHERE a.rowid=? AND a.server_id='default' \
                     AND c.server_id='default' AND c.is_default=1 \
                     AND c.name IN ('#general','#random'))",
                )
                .bind(row_id)
                .fetch_one(&mut *conn)
                .await?
            } else if table == "conversations" {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM conversations cv JOIN channels c \
                     ON c.id=cv.channel_id WHERE cv.rowid=? AND cv.server_id='default' \
                     AND c.server_id='default' AND c.is_default=1 \
                     AND c.name IN ('#general','#random'))",
                )
                .bind(row_id)
                .fetch_one(&mut *conn)
                .await?
            } else {
                false
            };
            if !known_default {
                findings.push(RepairFinding {
                    code: "foreign_key_violation",
                    object_type: "row",
                    object_id: format!("{table}:{row_id}"),
                    detail: format!(
                        "references missing parent table {parent}; explicit repair required"
                    ),
                    blocks_upgrade: true,
                });
            }
        }
    }
    Ok(findings)
}

/// Read-only schema recognition, checksum validation, and repair assessment.
pub async fn migration_preflight(
    pool: &SqlitePool,
) -> Result<MigrationPreflightReport, MigrationError> {
    let mut conn = pool.acquire().await?;
    migration_preflight_connection(&mut conn).await
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct UserOverrideRepair {
    pub override_id: String,
    pub channel_id: String,
    pub previous_target_id: String,
    pub target_user_id: String,
    pub allow_bits: i64,
    pub deny_bits: i64,
    pub evidence: String,
}

/// Apply one explicitly reviewed legacy user-override mapping.
///
/// The old row, chosen current user, permission bits, and operator evidence are
/// recorded in the durable migration repair log in the same exclusive
/// transaction as the mapping. This deliberately does not guess from handles.
pub async fn repair_user_override(
    pool: &SqlitePool,
    override_id: &str,
    target_user_id: &str,
    evidence: &str,
) -> Result<UserOverrideRepair, MigrationError> {
    if override_id.trim().is_empty() || target_user_id.trim().is_empty() {
        return Err(MigrationError::Integrity {
            check: "operator repair input",
            detail: "override and target user IDs must be non-empty".into(),
        });
    }
    if evidence.trim().is_empty() || evidence.len() > 2_000 {
        return Err(MigrationError::Integrity {
            check: "operator repair evidence",
            detail: "evidence must contain 1 to 2000 bytes".into(),
        });
    }
    let mut transaction = pool.begin_with("BEGIN EXCLUSIVE").await?;
    let report = migration_preflight_connection(&mut transaction).await?;
    let version = report.source_version;
    if version < 14 {
        return Err(MigrationError::Integrity {
            check: "operator repair provenance",
            detail: "post-014 identity repair requires schema version 14 or newer".into(),
        });
    }
    let row: Option<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT channel_id,target_id,allow_bits,deny_bits \
         FROM channel_permission_overrides WHERE id=? AND target_type='user'",
    )
    .bind(override_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let (channel_id, previous_target_id, allow_bits, deny_bits) =
        row.ok_or(MigrationError::Integrity {
            check: "operator repair target",
            detail: "user override does not exist".into(),
        })?;
    let previous_is_current: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=?)")
            .bind(&previous_target_id)
            .fetch_one(&mut *transaction)
            .await?;
    if previous_is_current {
        return Err(MigrationError::Integrity {
            check: "operator repair target",
            detail: "override already names a current user and requires no identity repair".into(),
        });
    }
    let target_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=?)")
        .bind(target_user_id)
        .fetch_one(&mut *transaction)
        .await?;
    if !target_exists {
        return Err(MigrationError::Integrity {
            check: "operator repair mapping",
            detail: "chosen target user does not exist".into(),
        });
    }
    let repair = UserOverrideRepair {
        override_id: override_id.to_owned(),
        channel_id,
        previous_target_id,
        target_user_id: target_user_id.to_owned(),
        allow_bits,
        deny_bits,
        evidence: evidence.to_owned(),
    };
    let details = serde_json::to_string(&repair).map_err(|_| MigrationError::Integrity {
        check: "operator repair evidence",
        detail: "repair evidence could not be encoded".into(),
    })?;
    if version < 17 {
        sqlx::query("PRAGMA defer_foreign_keys=ON")
            .execute(&mut *transaction)
            .await?;
        for migration in MIGRATIONS
            .iter()
            .copied()
            .filter(|migration| migration.version > version && migration.version <= 17)
        {
            sqlx::raw_sql(migration.sql)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("INSERT OR IGNORE INTO schema_version(version) VALUES(?)")
                .bind(migration.version)
                .execute(&mut *transaction)
                .await?;
        }
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 17)
        {
            sqlx::query(
                "INSERT INTO migration_metadata(version,checksum_sha256,provenance) \
                 VALUES(?,?,?)",
            )
            .bind(migration.version)
            .bind(checksum(migration.sql))
            .bind(if migration.version <= version {
                "adopted_release_effects"
            } else {
                "bundled_script"
            })
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO database_metadata(singleton,compatibility_floor,generation) \
             VALUES(1,?,?)",
        )
        .bind(COMPATIBILITY_FLOOR)
        .bind(Uuid::new_v4().to_string())
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "INSERT INTO migration_repair_log( \
            migration_version,repair_kind,object_type,object_id,outcome,details \
         ) VALUES(?,'post014_user_override','channel_permission_override',?,'operator_mapped',?)",
    )
    .bind(version.max(17))
    .bind(override_id)
    .bind(details)
    .execute(&mut *transaction)
    .await?;
    let updated = sqlx::query(
        "UPDATE channel_permission_overrides SET target_id=? \
         WHERE id=? AND target_type='user' AND target_id=?",
    )
    .bind(target_user_id)
    .bind(override_id)
    .bind(&repair.previous_target_id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(MigrationError::Integrity {
            check: "operator repair race",
            detail: "override changed during repair".into(),
        });
    }
    transaction.commit().await?;
    Ok(repair)
}

async fn migration_preflight_connection(
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

async fn capture_snapshot(conn: &mut SqliteConnection) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts%' ORDER BY name").fetch_all(&mut *conn).await?;
    let mut snapshot = Vec::with_capacity(tables.len());
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{quoted}\""))
            .fetch_one(&mut *conn)
            .await?;
        snapshot.push((table, count));
    }
    Ok(snapshot)
}

async fn record_snapshot(
    conn: &mut SqliteConnection,
    snapshot: &[(String, i64)],
) -> Result<(), sqlx::Error> {
    for (table, count) in snapshot {
        sqlx::query("INSERT INTO migration_snapshots(migration_version,table_name,row_count) VALUES (?,?,?)")
            .bind(LATEST_SCHEMA_VERSION).bind(table).bind(count).execute(&mut *conn).await?;
    }
    Ok(())
}

async fn apply_safe_repairs(
    conn: &mut SqliteConnection,
    findings: &[RepairFinding],
) -> Result<(), MigrationError> {
    for finding in findings
        .iter()
        .filter(|item| !item.blocks_upgrade && item.code == "legacy_default_server")
    {
        if object_exists(conn, "table", "conversations").await? {
            sqlx::query("DELETE FROM conversations WHERE channel_id=?")
                .bind(&finding.object_id)
                .execute(&mut *conn)
                .await?;
        }
        sqlx::query("DELETE FROM channels WHERE id=? AND server_id='default' AND is_default=1 AND name IN ('#general','#random') AND NOT EXISTS(SELECT 1 FROM messages WHERE channel_id=channels.id) AND NOT EXISTS(SELECT 1 FROM channel_members WHERE channel_id=channels.id)")
            .bind(&finding.object_id).execute(&mut *conn).await?;
        sqlx::query("INSERT INTO migration_repair_log(migration_version,repair_kind,object_type,object_id,outcome,details) VALUES (17,?,?,?,'repaired',?)")
            .bind(finding.code).bind(finding.object_type).bind(&finding.object_id).bind(&finding.detail).execute(&mut *conn).await?;
    }
    Ok(())
}

async fn apply_notification_scope_repairs(
    conn: &mut SqliteConnection,
    findings: &[RepairFinding],
) -> Result<(), MigrationError> {
    if !findings
        .iter()
        .any(|finding| finding.code == "duplicate_notification_scope" && !finding.blocks_upgrade)
    {
        return Ok(());
    }
    let scopes = sqlx::query(
        "SELECT user_id,server_id,channel_id FROM notification_settings \
         GROUP BY user_id,server_id,channel_id HAVING count(*)>1",
    )
    .fetch_all(&mut *conn)
    .await?;
    for scope in scopes {
        let user_id: String = scope.get(0);
        let server_id: Option<String> = scope.get(1);
        let channel_id: Option<String> = scope.get(2);
        let winner_id: String = sqlx::query_scalar(
            "SELECT id FROM notification_settings \
             WHERE user_id=? AND server_id IS ? AND channel_id IS ? \
             ORDER BY julianday(updated_at) DESC,updated_at DESC,id DESC LIMIT 1",
        )
        .bind(&user_id)
        .bind(&server_id)
        .bind(&channel_id)
        .fetch_one(&mut *conn)
        .await?;
        let exported: String = sqlx::query_scalar(
            "SELECT json_group_array(json_object( \
                'id',id,'level',level,'suppress_everyone',suppress_everyone, \
                'suppress_roles',suppress_roles,'muted',muted, \
                'mute_until',mute_until,'created_at',created_at,'updated_at',updated_at \
             )) FROM ( \
                SELECT * FROM notification_settings \
                WHERE user_id=? AND server_id IS ? AND channel_id IS ? \
                ORDER BY updated_at,id \
             )",
        )
        .bind(&user_id)
        .bind(&server_id)
        .bind(&channel_id)
        .fetch_one(&mut *conn)
        .await?;
        let exported: serde_json::Value =
            serde_json::from_str(&exported).map_err(|_| MigrationError::Integrity {
                check: "notification repair export",
                detail: "duplicate notification rows could not be encoded".into(),
            })?;
        let details = serde_json::json!({
            "user_id": user_id,
            "server_id": server_id,
            "channel_id": channel_id,
            "winner_id": winner_id,
            "selection": "latest valid updated_at, then greatest stable id",
            "pre_repair_rows": exported,
        });
        sqlx::query(
            "INSERT INTO migration_repair_log( \
                migration_version,repair_kind,object_type,object_id,outcome,details \
             ) VALUES(27,'duplicate_notification_scope','notification_settings',?, \
                      'deduplicated',?)",
        )
        .bind(&winner_id)
        .bind(details.to_string())
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "DELETE FROM notification_settings \
             WHERE user_id=? AND server_id IS ? AND channel_id IS ? AND id<>?",
        )
        .bind(&user_id)
        .bind(&server_id)
        .bind(&channel_id)
        .bind(&winner_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn verify_integrity(conn: &mut SqliteConnection) -> Result<(), MigrationError> {
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *conn)
        .await?;
    if !violations.is_empty() {
        return Err(MigrationError::Integrity {
            check: "foreign_key_check",
            detail: format!("{} violation(s) remain", violations.len()),
        });
    }
    let result: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut *conn)
        .await?;
    if result != "ok" {
        return Err(MigrationError::Integrity {
            check: "integrity_check",
            detail: result,
        });
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_schema_version_matches_registered_migration_tail() {
        assert_eq!(
            LATEST_SCHEMA_VERSION,
            MIGRATIONS.last().expect("at least one migration").version
        );
    }

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

    #[tokio::test]
    async fn fresh_database_has_verified_history_and_durable_profile() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM schema_version ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(versions, (1..=current_schema_version()).collect::<Vec<_>>());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM migration_metadata")
                .fetch_one(&pool)
                .await
                .unwrap(),
            current_schema_version()
        );
        let profile = (
            sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                .fetch_one(&pool)
                .await
                .unwrap(),
            sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
                .fetch_one(&pool)
                .await
                .unwrap(),
        );
        assert_eq!(profile, (1, 2));
        let snapshots_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM migration_snapshots")
            .fetch_one(&pool)
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let snapshots_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM migration_snapshots")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(snapshots_after, snapshots_before);
    }

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
    async fn full_schema_drift_is_rejected_without_upgrade_mutation() {
        let pool = historical_fixture(16).await;
        sqlx::query("ALTER TABLE reactions ADD COLUMN unrecognized_drift TEXT")
            .execute(&pool)
            .await
            .unwrap();
        let before: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='reactions'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let error = run_migrations(&pool).await.unwrap_err().to_string();
        assert!(error.contains("schema fingerprint"), "{error}");
        let after: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='reactions'",
        )
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
    async fn populated_default_orphan_returns_repair_report_without_mutation() {
        let pool = historical_fixture(2).await;
        let mut fixture_conn = pool.acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *fixture_conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('legacy-message','default','#general','legacy-user-2','legacy2','preserve me')")
            .execute(&mut *fixture_conn).await.unwrap();
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&mut *fixture_conn)
            .await
            .unwrap();
        drop(fixture_conn);
        let error = run_migrations(&pool).await.unwrap_err().to_string();
        assert!(
            error.contains("operator server mapping required"),
            "{error}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE id='legacy-message'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
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
    async fn unresolved_migration_014_user_override_remains_denied_and_reported() {
        let pool = historical_fixture(14).await;
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES('server','Server','legacy-user-14')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#safe')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id) VALUES('override','channel','user','old-uuid')").execute(&pool).await.unwrap();
        let error = run_migrations(&pool).await.unwrap_err().to_string();
        assert!(
            error.contains("audited identity mapping required"),
            "{error}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "old-uuid"
        );
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
    async fn operator_user_override_repair_preserves_evidence_and_permission_bits() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('mapped','mapped')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
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
             ) VALUES('override','channel','user','legacy-uuid',17,4)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let inventory = migration_preflight(&pool).await.unwrap();
        assert!(inventory.findings.iter().any(|finding| {
            finding.code == "unresolved_user_override" && finding.object_id == "override"
        }));
        let repair = repair_user_override(
            &pool,
            "override",
            "mapped",
            "operator ticket MIG-42 verified account ownership",
        )
        .await
        .unwrap();
        assert_eq!(repair.previous_target_id, "legacy-uuid");
        assert_eq!((repair.allow_bits, repair.deny_bits), (17, 4));
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "mapped"
        );
        let evidence: String = sqlx::query_scalar(
            "SELECT details FROM migration_repair_log \
             WHERE repair_kind='post014_user_override' AND object_id='override'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(evidence.contains("legacy-uuid"));
        assert!(evidence.contains("MIG-42"));
        assert!(!migration_preflight(&pool).await.unwrap().is_blocked());
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

    #[tokio::test]
    async fn ambiguous_pre014_identity_and_override_collision_are_not_guessed() {
        let pool = historical_fixture(13).await;
        sqlx::query(
            "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES \
             ('at-one','legacy-user-13','atproto','did:plc:one'), \
             ('at-two','legacy-user-13','atproto','did:plc:two')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) \
             VALUES('server','Server','legacy-user-13')",
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
             ) VALUES \
                ('legacy-override','channel','user','legacy-user-13',5,2), \
                ('current-override','channel','user','did:plc:one',8,1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let report = migration_preflight(&pool).await.unwrap();
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ambiguous_pre014_at_identity" && finding.object_id == "legacy-user-13"
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.code == "pre014_override_target_collision"
                && finding.object_id == "legacy-override"
        }));
        assert!(run_migrations(&pool).await.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT target_id FROM channel_permission_overrides \
                 WHERE id='legacy-override'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "legacy-user-13"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_version")
                .fetch_one(&pool)
                .await
                .unwrap(),
            13
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

    #[tokio::test]
    async fn audit_actor_snapshot_survives_actor_account_deletion() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username,avatar_url) VALUES \
             ('owner','owner',NULL),('moderator','mod-at-action','avatar-at-action')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        crate::db::queries::audit_log::create_entry(
            &pool,
            &crate::db::models::CreateAuditLogParams {
                id: "audit",
                server_id: "server",
                actor_id: "moderator",
                action_type: "member_kick",
                target_type: Some("user"),
                target_id: Some("target"),
                reason: None,
                changes: None,
            },
        )
        .await
        .unwrap();
        sqlx::query("DELETE FROM users WHERE id='moderator'")
            .execute(&pool)
            .await
            .unwrap();
        let snapshot: (String, String, Option<String>) = sqlx::query_as(
            "SELECT actor_id,actor_username_snapshot,actor_avatar_snapshot \
             FROM audit_log WHERE id='audit'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            snapshot,
            (
                "moderator".into(),
                "mod-at-action".into(),
                Some("avatar-at-action".into())
            )
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
}
