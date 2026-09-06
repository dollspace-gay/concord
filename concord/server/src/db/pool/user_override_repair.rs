use super::{
    COMPATIBILITY_FLOOR, MIGRATIONS, MigrationError, SqlitePool, Uuid, checksum,
    migration_preflight_connection,
};

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
