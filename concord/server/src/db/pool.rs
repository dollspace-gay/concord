use sha2::{Digest, Sha256};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use sqlx::{Row, SqliteConnection, SqlitePool};

use std::{fmt, str::FromStr, time::Duration};

use tracing::info;

use uuid::Uuid;

const LATEST_SCHEMA_VERSION: i64 = 32;

const COMPATIBILITY_FLOOR: i64 = 17;

pub const fn current_schema_version() -> i64 {
    LATEST_SCHEMA_VERSION
}

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

/// Read-only schema recognition, checksum validation, and repair assessment.
pub async fn migration_preflight(
    pool: &SqlitePool,
) -> Result<MigrationPreflightReport, MigrationError> {
    let mut conn = pool.acquire().await?;
    migration_preflight_connection(&mut conn).await
}

#[cfg(test)]
mod tests;

mod catalog;
mod introspection;
mod migration_runner;
mod preflight;
mod repair_inspection;
mod repairs;
mod user_override_repair;
use catalog::MIGRATIONS;

use introspection::checksum;
use introspection::column_exists;
use introspection::expected_fingerprint;
use introspection::object_exists;
use introspection::schema_fingerprint;
use introspection::source_version;
pub use migration_runner::run_migrations;
use preflight::capture_snapshot;
use preflight::migration_preflight_connection;
use preflight::record_snapshot;
use repair_inspection::inspect_repairs;
use repairs::apply_notification_scope_repairs;
use repairs::apply_safe_repairs;
use repairs::verify_integrity;
pub use user_override_repair::UserOverrideRepair;
pub use user_override_repair::repair_user_override;
