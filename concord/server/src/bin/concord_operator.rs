use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use uuid::Uuid;

use concord_server::config::ServerConfig;
use concord_server::db::pool::{
    create_pool, migration_preflight, repair_user_override, run_migrations,
};
use concord_server::secrets::{
    SecretVault, migrate_legacy_atproto_credentials, rotate_external_envelopes,
};

#[derive(Parser)]
#[command(name = "concord-operator")]
struct Cli {
    #[arg(long, default_value = "concord.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    KeyInit {
        #[arg(long)]
        key_file: PathBuf,
    },
    SecretsMigrate,
    SecretsRotate {
        #[arg(long)]
        new_key_file: PathBuf,
    },
    MediaInventory,
    MediaRetry {
        attachment_id: String,
    },
    MediaImport {
        #[arg(long, default_value_t = 300)]
        lease_seconds: i64,
    },
    /// List durable AT publication state without contacting a provider.
    AtprotoPublicationInventory,
    /// Requeue one failed/uncertain publication after current-policy checks.
    AtprotoPublicationReconcile {
        publication_id: String,
    },
    MigrationInventory,
    /// Report the recognized source and target schema without applying changes.
    MigrationStatus,
    /// Apply all recognized migrations after a successful repair preflight.
    MigrationApply,
    MigrationRepairUserOverride {
        #[arg(long)]
        override_id: String,
        #[arg(long)]
        target_user_id: String,
        #[arg(long)]
        evidence: String,
    },
    /// List current system administrators by stable user ID.
    AdminInventory,
    /// Atomically transfer system administration between verified human users.
    AdminTransfer {
        #[arg(long)]
        from_user_id: String,
        #[arg(long)]
        to_user_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Add a verified human user as an administrator for local recovery.
    AdminRecover {
        #[arg(long)]
        user_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Revoke every active local credential for a verified human user.
    CredentialRevokeAll {
        #[arg(long)]
        user_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Inspect bounded external-job status without printing payloads or grants.
    JobsInspect {
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
    /// Requeue one failed external job; its dispatcher revalidates source policy.
    JobRetry {
        job_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Create a stopped-service database, media, configuration, and key backup.
    BackupCreate {
        #[arg(long)]
        destination: PathBuf,
    },
    /// Verify checksums, database integrity/schema, media references, and keys.
    BackupVerify {
        #[arg(long)]
        backup: PathBuf,
    },
    /// Restore a verified backup into the empty paths named by --config.
    BackupRestore {
        #[arg(long)]
        backup: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let restore_started = matches!(&cli.command, Command::BackupRestore { .. }).then(Instant::now);
    match run(cli).await {
        Ok(()) => {
            if let Some(started) = restore_started {
                println!("{}", operation_result("restore", "success", started));
            }
        }
        Err(error) => {
            if let Some(started) = restore_started {
                eprintln!("{}", operation_result("restore", "failure", started));
            }
            eprintln!("operator command failed: {error:#}");
            std::process::exit(1);
        }
    }
}

fn operation_result(operation: &str, outcome: &str, started: Instant) -> String {
    serde_json::json!({
        "kind": "concord_operator_operation",
        "operation": operation,
        "outcome": outcome,
        "duration_seconds": started.elapsed().as_secs_f64(),
    })
    .to_string()
}

async fn run(cli: Cli) -> Result<()> {
    if let Command::KeyInit { key_file } = &cli.command {
        write_new_key(key_file)?;
        println!("initialized external credential key {}", key_file.display());
        return Ok(());
    }
    if let Command::BackupVerify { backup } = &cli.command {
        let manifest = verify_backup(backup).await?;
        println!(
            "backup_verified={} schema_version={} files={}",
            manifest.backup_id,
            manifest.schema_version,
            manifest.files.len()
        );
        return Ok(());
    }
    let recovery = matches!(
        &cli.command,
        Command::MediaInventory
            | Command::MediaRetry { .. }
            | Command::MediaImport { .. }
            | Command::MigrationInventory
            | Command::MigrationStatus
            | Command::MigrationApply
            | Command::AtprotoPublicationInventory
            | Command::AtprotoPublicationReconcile { .. }
            | Command::MigrationRepairUserOverride { .. }
            | Command::AdminInventory
            | Command::AdminTransfer { .. }
            | Command::AdminRecover { .. }
            | Command::CredentialRevokeAll { .. }
            | Command::JobsInspect { .. }
            | Command::JobRetry { .. }
            | Command::BackupCreate { .. }
            | Command::BackupVerify { .. }
            | Command::BackupRestore { .. }
    );
    let config = if recovery {
        ServerConfig::load_for_recovery(&cli.config)
    } else {
        ServerConfig::load(&cli.config)
    }
    .with_context(|| format!("load configuration {}", cli.config.display()))?;
    let _maintenance =
        concord_server::operations::acquire_database_exclusion(&config.database.url)?;
    if let Command::BackupRestore { backup } = &cli.command {
        let manifest = verify_backup(backup).await?;
        restore_backup(&config, backup, &manifest).await?;
        println!(
            "backup_restored={} activation_required=true external_jobs_paused=true",
            manifest.backup_id
        );
        return Ok(());
    }
    let pool = create_pool(&config.database.url).await?;
    match cli.command {
        Command::KeyInit { .. } => unreachable!(),
        Command::SecretsMigrate => {
            run_migrations(&pool).await?;
            let vault = SecretVault::load(&config.auth.external_credentials_key_file)?;
            let report = migrate_legacy_atproto_credentials(&pool, &vault).await?;
            println!(
                "migrated_accounts={} missing_accounts={} migrated_signing_keys={}",
                report.migrated, report.missing_data, report.signing_keys
            );
        }
        Command::SecretsRotate { new_key_file } => {
            run_migrations(&pool).await?;
            let vault = SecretVault::load(&config.auth.external_credentials_key_file)?;
            type RotationState = (String, String, String, String, String);
            let mut rotation: Option<RotationState> = sqlx::query_as(
                "SELECT old_key_id,new_key_id,old_key_backup,durable_replacement,phase
                 FROM credential_rotation_state WHERE singleton=1",
            )
            .fetch_optional(&pool)
            .await?;
            if let Some((_, new_key_id, backup, durable, phase)) = rotation.clone() {
                if phase == "database_committed" {
                    let durable_vault = SecretVault::load(Path::new(&durable))?;
                    if durable_vault.key_id() != new_key_id {
                        bail!("durable replacement key does not match rotation record");
                    }
                    activate_key(
                        Path::new(&durable),
                        &config.auth.external_credentials_key_file,
                    )?;
                    sqlx::query(
                        "UPDATE credential_rotation_state
                         SET phase='activated',updated_at=datetime('now')
                         WHERE singleton=1 AND new_key_id=? AND phase='database_committed'",
                    )
                    .bind(&new_key_id)
                    .execute(&pool)
                    .await?;
                    println!(
                        "recovered_committed_rotation={} old_key_backup={}",
                        new_key_id, backup
                    );
                    return Ok(());
                }
                if phase == "activated" {
                    if vault.key_id() != new_key_id {
                        bail!("rotation is recorded activated but the active key does not match");
                    }
                    if new_key_file.exists() {
                        let requested = SecretVault::load(&new_key_file)?;
                        if requested.key_id() == new_key_id {
                            println!("credential rotation already activated key_id={new_key_id}");
                            return Ok(());
                        }
                        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
                        sqlx::query(
                            "INSERT OR IGNORE INTO credential_rotation_history
                             (old_key_id,new_key_id,old_key_backup,durable_replacement,started_at,activated_at)
                             SELECT old_key_id,new_key_id,old_key_backup,durable_replacement,
                                    started_at,updated_at
                             FROM credential_rotation_state WHERE singleton=1 AND phase='activated'",
                        )
                        .execute(&mut *transaction)
                        .await?;
                        sqlx::query("DELETE FROM credential_rotation_state WHERE singleton=1 AND phase='activated'")
                            .execute(&mut *transaction).await?;
                        transaction.commit().await?;
                        rotation = None;
                    } else {
                        println!("credential rotation already activated key_id={new_key_id}");
                        return Ok(());
                    }
                }
            }
            let replacement_source = rotation
                .as_ref()
                .map(|(_, _, _, durable, _)| Path::new(durable))
                .unwrap_or(new_key_file.as_path());
            let replacement = SecretVault::load(replacement_source)?;
            if let Some((_, new_key_id, _, _, _)) = &rotation
                && new_key_id != replacement.key_id()
            {
                bail!("a different credential key rotation is already in progress");
            }
            let backup =
                backup_current_key(&config.auth.external_credentials_key_file, vault.key_id())?;
            let durable_replacement = backup_replacement_key(
                replacement_source,
                &config.auth.external_credentials_key_file,
                replacement.key_id(),
            )?;
            match rotation {
                Some((old_key_id, _, recorded_backup, recorded_replacement, phase)) => {
                    if phase != "prepared"
                        || old_key_id != vault.key_id()
                        || recorded_backup != backup.to_string_lossy()
                        || recorded_replacement != durable_replacement.to_string_lossy()
                    {
                        bail!("prepared credential rotation does not match durable key copies");
                    }
                }
                None => {
                    sqlx::query(
                        "INSERT INTO credential_rotation_state
                         (singleton,old_key_id,new_key_id,old_key_backup,durable_replacement,phase)
                         VALUES(1,?,?,?,?,'prepared')",
                    )
                    .bind(vault.key_id())
                    .bind(replacement.key_id())
                    .bind(backup.to_string_lossy().as_ref())
                    .bind(durable_replacement.to_string_lossy().as_ref())
                    .execute(&pool)
                    .await?;
                }
            }
            let report = rotate_external_envelopes(&pool, &vault, &replacement).await?;
            rotation_test_barrier("database-committed");
            activate_key(
                &durable_replacement,
                &config.auth.external_credentials_key_file,
            )
            .with_context(|| {
                format!(
                    "database rotation committed; activate {} manually; old key backup is {}",
                    durable_replacement.display(),
                    backup.display()
                )
            })?;
            rotation_test_barrier("key-activated");
            sqlx::query(
                "UPDATE credential_rotation_state
                 SET phase='activated',updated_at=datetime('now')
                 WHERE singleton=1 AND new_key_id=? AND phase='database_committed'",
            )
            .bind(replacement.key_id())
            .execute(&pool)
            .await?;
            println!(
                "rotated_accounts={} rotated_signing_keys={} rotated_pending_oauth={} old_key_backup={}",
                report.accounts,
                report.signing_keys,
                report.pending_oauth,
                backup.display()
            );
        }
        Command::AtprotoPublicationInventory => {
            run_migrations(&pool).await?;
            let rows = sqlx::query(
                "SELECT p.id,p.user_id,p.source_message_id,p.source_version,p.status,
                        p.remote_uri,p.safe_error_code,p.updated_at
                 FROM atproto_publications p ORDER BY p.updated_at,p.id",
            )
            .fetch_all(&pool)
            .await?;
            for row in rows {
                use sqlx::Row;
                println!(
                    "{}",
                    serde_json::json!({
                        "id": row.get::<String,_>(0), "user_id": row.get::<String,_>(1),
                        "source_message_id": row.get::<String,_>(2), "source_version": row.get::<i64,_>(3),
                        "status": row.get::<String,_>(4), "remote_uri": row.get::<Option<String>,_>(5),
                        "safe_error_code": row.get::<Option<String>,_>(6), "updated_at": row.get::<String,_>(7),
                    })
                );
            }
        }
        Command::AtprotoPublicationReconcile { publication_id } => {
            run_migrations(&pool).await?;
            let status = reconcile_atproto_publication(&pool, &publication_id).await?;
            println!("publication_requeued={} status={}", publication_id, status);
        }
        Command::MediaInventory => {
            for row in concord_server::media::external_reference_inventory(&pool).await? {
                println!("{}", serde_json::to_string(&row)?);
            }
        }
        Command::MediaRetry { attachment_id } => {
            if !concord_server::media::retry_legacy_import(&pool, &attachment_id).await? {
                bail!("attachment is not in a retryable import state");
            }
            println!("queued attachment {attachment_id}");
        }
        Command::MediaImport { lease_seconds } => {
            let egress = concord_server::egress::EgressServices::internet_with_admin_origins(
                &config.egress.operator_allowed_origins,
            )?;
            let report = concord_server::media::import_legacy_batch(
                &pool,
                &config.storage.media_dir,
                &egress.imports,
                config.storage.max_file_size_mb * 1024 * 1024,
                lease_seconds,
                1,
            )
            .await?;
            println!(
                "claimed={} imported={} unresolved={}",
                report.claimed, report.imported, report.unresolved
            );
        }
        Command::MigrationInventory => {
            let report = migration_preflight(&pool).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.is_blocked() {
                bail!("migration repair inventory contains blocking findings");
            }
        }
        Command::MigrationStatus => {
            let report = migration_preflight(&pool).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.is_blocked() {
                bail!("migration preflight contains blocking findings");
            }
        }
        Command::MigrationApply => {
            let before = migration_preflight(&pool).await?;
            if before.is_blocked() {
                println!("{}", serde_json::to_string_pretty(&before)?);
                bail!("migration preflight contains blocking findings");
            }
            run_migrations(&pool).await?;
            let after = migration_preflight(&pool).await?;
            println!(
                "migration_applied_from={} migration_current={} findings={}",
                before.source_version,
                after.source_version,
                after.findings.len()
            );
        }
        Command::MigrationRepairUserOverride {
            override_id,
            target_user_id,
            evidence,
        } => {
            let repaired =
                repair_user_override(&pool, &override_id, &target_user_id, &evidence).await?;
            println!("{}", serde_json::to_string_pretty(&repaired)?);
        }
        Command::AdminInventory => {
            run_migrations(&pool).await?;
            print_admin_inventory(&pool, &config.admin.admin_user_ids).await?;
        }
        Command::AdminTransfer {
            from_user_id,
            to_user_id,
            reason,
        } => {
            run_migrations(&pool).await?;
            transfer_admin(
                &pool,
                &config.admin.admin_user_ids,
                &from_user_id,
                &to_user_id,
                &reason,
            )
            .await?;
            println!("admin_transferred_from={from_user_id} admin_transferred_to={to_user_id}");
        }
        Command::AdminRecover { user_id, reason } => {
            run_migrations(&pool).await?;
            recover_admin(&pool, &user_id, &reason).await?;
            println!("admin_recovered={user_id}");
        }
        Command::CredentialRevokeAll { user_id, reason } => {
            run_migrations(&pool).await?;
            let revoked = revoke_all_user_credentials(&pool, &user_id, &reason).await?;
            println!("credential_user={user_id} credentials_revoked={revoked}");
        }
        Command::JobsInspect { state, limit } => {
            run_migrations(&pool).await?;
            print_external_jobs(&pool, state.as_deref(), limit).await?;
        }
        Command::JobRetry { job_id, reason } => {
            run_migrations(&pool).await?;
            retry_external_job(&pool, &job_id, &reason).await?;
            println!("external_job_requeued={job_id}");
        }
        Command::BackupCreate { destination } => {
            create_backup(&pool, &config, &cli.config, &destination).await?;
            println!("backup_created={}", destination.display());
        }
        Command::BackupVerify { .. } => unreachable!(),
        Command::BackupRestore { .. } => unreachable!(),
    }
    Ok(())
}

fn validate_operator_reason(reason: &str) -> Result<&str> {
    let reason = reason.trim();
    if reason.is_empty() || reason.len() > 1000 {
        bail!("operator reason must contain between 1 and 1000 bytes");
    }
    Ok(reason)
}

async fn require_verified_human(
    connection: &mut sqlx::SqliteConnection,
    user_id: &str,
    allow_disabled: bool,
) -> Result<()> {
    let row: Option<(i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT u.is_bot,u.disabled_at,EXISTS( \
           SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=u.id \
             AND oa.provider='atproto' AND length(trim(oa.provider_id))>0) \
         FROM users u WHERE u.id=?",
    )
    .bind(user_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((is_bot, disabled_at, has_verified_identity)) = row else {
        bail!("stable user ID was not found: {user_id}");
    };
    if is_bot != 0 {
        bail!("operator recovery requires a human user ID");
    }
    if disabled_at.is_some() && !allow_disabled {
        bail!("operator recovery target is disabled");
    }
    if has_verified_identity != 1 {
        bail!("operator recovery target has no verified AT Protocol identity mapping");
    }
    Ok(())
}

async fn insert_operator_audit(
    connection: &mut sqlx::SqliteConnection,
    action_type: &str,
    target_type: &str,
    target_id: &str,
    reason: &str,
    details: &serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO operator_audit_log( \
           id,action_type,target_type,target_id,reason,details_json) \
         VALUES(?,?,?,?,?,?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(action_type)
    .bind(target_type)
    .bind(target_id)
    .bind(reason)
    .bind(serde_json::to_string(details)?)
    .execute(connection)
    .await?;
    Ok(())
}

async fn print_admin_inventory(pool: &sqlx::SqlitePool, configured_ids: &[String]) -> Result<()> {
    let rows: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT u.id,u.username,u.disabled_at,EXISTS( \
           SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=u.id \
             AND oa.provider='atproto' AND length(trim(oa.provider_id))>0) \
         FROM users u WHERE u.is_system_admin=1 ORDER BY u.id",
    )
    .fetch_all(pool)
    .await?;
    let admins: Vec<_> = rows
        .into_iter()
        .map(|(user_id, username, disabled_at, verified_identity)| {
            let configured_bootstrap = configured_ids.iter().any(|item| item == &user_id);
            serde_json::json!({
                "user_id": user_id,
                "username": username,
                "disabled": disabled_at.is_some(),
                "verified_atproto_identity": verified_identity == 1,
                "configured_bootstrap": configured_bootstrap,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&admins)?);
    Ok(())
}

async fn transfer_admin(
    pool: &sqlx::SqlitePool,
    configured_ids: &[String],
    from_user_id: &str,
    to_user_id: &str,
    reason: &str,
) -> Result<()> {
    let reason = validate_operator_reason(reason)?;
    if from_user_id == to_user_id {
        bail!("administrator transfer requires two different stable user IDs");
    }
    if configured_ids.iter().any(|item| item == from_user_id) {
        bail!(
            "remove {from_user_id} from admin.admin_user_ids and validate the configuration before transferring; otherwise a later verified login would restore that privilege"
        );
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_verified_human(&mut transaction, from_user_id, false).await?;
    require_verified_human(&mut transaction, to_user_id, false).await?;
    let source_is_admin: i64 = sqlx::query_scalar("SELECT is_system_admin FROM users WHERE id=?")
        .bind(from_user_id)
        .fetch_one(&mut *transaction)
        .await?;
    if source_is_admin != 1 {
        bail!("transfer source is not a current administrator");
    }
    sqlx::query("UPDATE users SET is_system_admin=1 WHERE id=?")
        .bind(to_user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE users SET is_system_admin=0 WHERE id=? AND is_system_admin=1")
        .bind(from_user_id)
        .execute(&mut *transaction)
        .await?;
    insert_operator_audit(
        &mut transaction,
        "admin_transfer",
        "user",
        to_user_id,
        reason,
        &serde_json::json!({"from_user_id": from_user_id, "to_user_id": to_user_id}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn recover_admin(pool: &sqlx::SqlitePool, user_id: &str, reason: &str) -> Result<()> {
    let reason = validate_operator_reason(reason)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_verified_human(&mut transaction, user_id, false).await?;
    let changed =
        sqlx::query("UPDATE users SET is_system_admin=1 WHERE id=? AND is_system_admin=0")
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
    if changed.rows_affected() != 1 {
        bail!("recovery target is already a current administrator");
    }
    insert_operator_audit(
        &mut transaction,
        "admin_recovery",
        "user",
        user_id,
        reason,
        &serde_json::json!({"user_id": user_id}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn revoke_all_user_credentials(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    reason: &str,
) -> Result<u64> {
    let reason = validate_operator_reason(reason)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_verified_human(&mut transaction, user_id, true).await?;
    let local_credentials = sqlx::query(
        "UPDATE auth_credentials SET revoked_at=unixepoch(),version=version+1 \
         WHERE user_id=? AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let delegated_tokens = sqlx::query(
        "UPDATE oauth2_tokens SET revoked_at=COALESCE(revoked_at,datetime('now')) \
         WHERE grant_id IN(SELECT id FROM oauth2_grants WHERE user_id=?) \
           AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let delegated_grants = sqlx::query(
        "UPDATE oauth2_grants SET state='revoked',revoked_at=datetime('now'), \
                grant_version=grant_version+1 \
         WHERE user_id=? AND state='active'",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let authorization_codes = sqlx::query(
        "UPDATE oauth2_codes SET consumed_at=COALESCE(consumed_at,datetime('now')) \
         WHERE user_id=? AND consumed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let consent_requests = sqlx::query(
        "UPDATE oauth2_consent_requests \
         SET consumed_at=COALESCE(consumed_at,datetime('now')) \
         WHERE user_id=? AND consumed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let revoked = local_credentials.rows_affected()
        + delegated_tokens.rows_affected()
        + delegated_grants.rows_affected()
        + authorization_codes.rows_affected()
        + consent_requests.rows_affected();
    insert_operator_audit(
        &mut transaction,
        "credential_revoke_all",
        "user",
        user_id,
        reason,
        &serde_json::json!({
            "user_id": user_id,
            "local_credentials": local_credentials.rows_affected(),
            "delegated_tokens": delegated_tokens.rows_affected(),
            "delegated_grants": delegated_grants.rows_affected(),
            "authorization_codes": authorization_codes.rows_affected(),
            "consent_requests": consent_requests.rows_affected(),
        }),
    )
    .await?;
    transaction.commit().await?;
    Ok(revoked)
}

async fn print_external_jobs(
    pool: &sqlx::SqlitePool,
    state: Option<&str>,
    limit: i64,
) -> Result<()> {
    const STATES: &[&str] = &["pending", "leased", "succeeded", "failed", "cancelled"];
    if !(1..=500).contains(&limit) {
        bail!("job inspection limit must be between 1 and 500");
    }
    if state.is_some_and(|value| !STATES.contains(&value)) {
        bail!("job state filter is invalid");
    }
    type JobInventoryRow = (
        String,
        String,
        String,
        i64,
        String,
        i64,
        Option<String>,
        String,
        String,
    );
    let rows: Vec<JobInventoryRow> = if let Some(state) = state {
        sqlx::query_as(
            "SELECT id,operation_type,resource_id,resource_version,state,attempt_count, \
                    safe_error_code,next_attempt_at,updated_at \
             FROM external_jobs WHERE state=? ORDER BY updated_at,id LIMIT ?",
        )
        .bind(state)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT id,operation_type,resource_id,resource_version,state,attempt_count, \
                    safe_error_code,next_attempt_at,updated_at \
             FROM external_jobs ORDER BY updated_at,id LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    for row in rows {
        println!(
            "{}",
            serde_json::json!({
                "id": row.0,
                "operation_type": row.1,
                "resource_id": row.2,
                "resource_version": row.3,
                "state": row.4,
                "attempt_count": row.5,
                "safe_error_code": row.6,
                "next_attempt_at": row.7,
                "updated_at": row.8,
            })
        );
    }
    Ok(())
}

async fn retry_external_job(pool: &sqlx::SqlitePool, job_id: &str, reason: &str) -> Result<()> {
    let reason = validate_operator_reason(reason)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: (String, String) = sqlx::query_as(
        "SELECT operation_type,resource_id FROM external_jobs WHERE id=? AND state='failed'",
    )
    .bind(job_id)
    .fetch_optional(&mut *transaction)
    .await?
    .with_context(|| format!("failed external job was not found: {job_id}"))?;
    if row.0 != "webhook_delivery" {
        if matches!(
            row.0.as_str(),
            "atproto_publish" | "atproto_update" | "atproto_delete"
        ) {
            bail!(
                "AT publication jobs require atproto-publication-reconcile so uncertain remote state and current grants are checked"
            );
        }
        bail!(
            "external job type is not eligible for operator retry: {}",
            row.0
        );
    }
    let eligible: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM webhook_deliveries d \
           JOIN webhooks w ON w.id=d.webhook_id \
           JOIN channels c ON c.id=w.channel_id AND c.server_id=w.server_id \
           WHERE d.external_job_id=? AND d.delivery_id=? AND d.state='failed' \
             AND w.webhook_type='outgoing' AND w.credential_state='active' \
             AND w.revoked_at IS NULL AND w.url IS NOT NULL AND c.is_private=0)",
    )
    .bind(job_id)
    .bind(&row.1)
    .fetch_one(&mut *transaction)
    .await?;
    if eligible != 1 {
        bail!("external job is no longer eligible under its current webhook grant");
    }
    let changed = sqlx::query(
        "UPDATE external_jobs SET state='pending',next_attempt_at=datetime('now'), \
                lease_owner=NULL,lease_token=NULL,lease_until=NULL,safe_error_code=NULL, \
                updated_at=datetime('now') \
         WHERE id=? AND state='failed'",
    )
    .bind(job_id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        bail!("failed external job changed while retry was admitted");
    }
    let delivery = sqlx::query(
        "UPDATE webhook_deliveries SET state='pending',last_status=NULL,safe_error_code=NULL \
         WHERE external_job_id=? AND delivery_id=? AND state='failed'",
    )
    .bind(job_id)
    .bind(&row.1)
    .execute(&mut *transaction)
    .await?;
    if delivery.rows_affected() != 1 {
        bail!("matching failed webhook delivery was not found");
    }
    insert_operator_audit(
        &mut transaction,
        "external_job_retry",
        "external_job",
        job_id,
        reason,
        &serde_json::json!({"operation_type": row.0, "resource_id": row.1}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn reconcile_atproto_publication(
    pool: &sqlx::SqlitePool,
    publication_id: &str,
) -> Result<String> {
    use sqlx::Row;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT p.user_id,p.source_version,p.remote_uri,m.deleted_at,m.sender_id,m.channel_id,
                g.grant_version,c.atproto_publication_enabled,c.is_private,
                c.visibility_repair_required,c.parent_channel_id,c.channel_type,
                EXISTS(SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=p.user_id
                  AND oa.provider='atproto' AND oa.credential_state='active')
         FROM atproto_publications p JOIN messages m ON m.id=p.source_message_id
         JOIN channels c ON c.id=m.channel_id
         LEFT JOIN atproto_publication_grants g ON g.user_id=p.user_id
              AND g.channel_id=m.channel_id AND g.enabled=1
         WHERE p.id=? AND p.status='failed'",
    )
    .bind(publication_id)
    .fetch_optional(&mut *transaction)
    .await?
    .with_context(|| format!("failed publication {publication_id} was not found"))?;
    let user_id: String = row.get(0);
    if row.get::<String, _>(4) != user_id || !row.get::<bool, _>(12) {
        bail!("publication source ownership or AT credential is no longer valid");
    }
    let deleted = row.get::<Option<String>, _>(3).is_some();
    let operation = if deleted {
        "atproto_delete"
    } else {
        let eligible = row.get::<Option<i64>, _>(6).is_some()
            && row.get::<i64, _>(7) == 1
            && row.get::<i64, _>(8) == 0
            && row.get::<i64, _>(9) == 0
            && row.get::<Option<String>, _>(10).is_none()
            && !matches!(
                row.get::<String, _>(11).as_str(),
                "public_thread" | "private_thread"
            );
        if !eligible {
            bail!("publication is no longer eligible under current channel/user grant");
        }
        if row.get::<Option<String>, _>(2).is_some() {
            "atproto_update"
        } else {
            "atproto_publish"
        }
    };
    let source_version: i64 = row.get(1);
    let status = match operation {
        "atproto_delete" => "delete_pending",
        "atproto_update" => "update_pending",
        _ => "pending",
    };
    let job_id = Uuid::new_v4().to_string();
    sqlx::query("UPDATE atproto_publications SET status=?,safe_error_code=NULL,updated_at=datetime('now') WHERE id=? AND status='failed'")
        .bind(status).bind(publication_id).execute(&mut *transaction).await?;
    sqlx::query(
        "INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,
          resource_version,destination_grant,payload_json) VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&job_id)
    .bind(format!(
        "atproto-publication:{publication_id}:{source_version}:operator:{job_id}"
    ))
    .bind(operation)
    .bind(publication_id)
    .bind(source_version)
    .bind(format!(
        "atproto-user:{user_id}:{}",
        row.get::<Option<i64>, _>(6).unwrap_or(0)
    ))
    .bind(
        serde_json::json!({"publication_id":publication_id,"reconcile":true,"operator":true})
            .to_string(),
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(status.to_owned())
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    format_version: u32,
    backup_id: String,
    created_at: String,
    schema_version: i64,
    database_generation: String,
    files: Vec<BackupFile>,
    external_credentials_key_id: String,
    has_jwt_secret_file: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupFile {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct StoredBackupEnvelope {
    key_id: String,
    ciphertext: String,
}

async fn create_backup(
    pool: &sqlx::SqlitePool,
    config: &ServerConfig,
    config_path: &Path,
    destination: &Path,
) -> Result<()> {
    let database = database_path(&config.database.url)?;
    let mut sources = vec![
        config.storage.media_dir.as_path(),
        config_path,
        config.auth.external_credentials_key_file.as_path(),
        database.as_path(),
    ];
    if let Some(jwt) = &config.auth.jwt_secret_file {
        sources.push(jwt);
    }
    reject_overlapping_destination(destination, &sources)?;
    create_private_destination(destination)?;

    let result = async {
        let report = migration_preflight(pool).await?;
        if report.is_blocked() {
            bail!("database migration preflight contains blocking findings");
        }
        let schema_version: i64 = sqlx::query_scalar("SELECT max(version) FROM schema_version")
            .fetch_one(pool)
            .await?;
        if schema_version != concord_server::db::pool::current_schema_version() {
            bail!("backup requires the current schema version");
        }
        let database_generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(pool)
                .await?;

        let database = destination.join("database.sqlite");
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(pool)
            .await?;
        sqlx::query("VACUUM INTO ?")
            .bind(database.to_string_lossy().as_ref())
            .execute(pool)
            .await
            .context("create coordinated SQLite snapshot")?;

        copy_tree(&config.storage.media_dir, &destination.join("media"))?;
        copy_private_file(config_path, &destination.join("config/source.toml"))?;
        copy_private_file(
            &config.auth.external_credentials_key_file,
            &destination.join("secrets/external-credentials.key"),
        )?;
        let has_jwt_secret_file = if let Some(jwt) = &config.auth.jwt_secret_file {
            copy_private_file(jwt, &destination.join("secrets/jwt-secret"))?;
            true
        } else {
            false
        };

        let key = SecretVault::load(&config.auth.external_credentials_key_file)?;
        let mut files = inventory_files(destination)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = BackupManifest {
            format_version: 1,
            backup_id: Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            schema_version,
            database_generation,
            files,
            external_credentials_key_id: key.key_id().to_owned(),
            has_jwt_secret_file,
        };
        write_private_new(
            &destination.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?.as_slice(),
        )?;
        verify_backup(destination).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

async fn verify_backup(backup: &Path) -> Result<BackupManifest> {
    let manifest_path = backup.join("manifest.json");
    let manifest: BackupManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.format_version != 1 {
        bail!("unsupported backup manifest version");
    }
    let actual_files = inventory_files(backup)?;
    if actual_files.len() != manifest.files.len()
        || actual_files.iter().any(|actual| {
            !manifest
                .files
                .iter()
                .any(|expected| expected.path == actual.path)
        })
    {
        bail!("backup contents do not exactly match the manifest");
    }
    for expected in &manifest.files {
        let path = checked_backup_path(backup, &expected.path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() || metadata.len() != expected.size {
            bail!("backup file size/type mismatch: {}", expected.path);
        }
        if sha256_file(&path)? != expected.sha256 {
            bail!("backup checksum mismatch: {}", expected.path);
        }
    }
    for required in [
        "database.sqlite",
        "config/source.toml",
        "secrets/external-credentials.key",
    ] {
        if !manifest.files.iter().any(|file| file.path == required) {
            bail!("backup manifest is missing {required}");
        }
    }
    if manifest.has_jwt_secret_file
        && !manifest
            .files
            .iter()
            .any(|file| file.path == "secrets/jwt-secret")
    {
        bail!("backup manifest is missing the JWT secret file");
    }
    let vault = SecretVault::load(&backup.join("secrets/external-credentials.key"))?;
    if vault.key_id() != manifest.external_credentials_key_id {
        bail!("backup external credential key does not match its manifest");
    }

    let options = SqliteConnectOptions::from_str(&format!(
        "sqlite:{}?mode=ro",
        backup.join("database.sqlite").display()
    ))?
    .read_only(true);
    let verify_pool = sqlx::SqlitePool::connect_with(options).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&verify_pool)
        .await?;
    if integrity != "ok" {
        bail!("backup database integrity check failed: {integrity}");
    }
    let foreign_key_failures: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(&verify_pool)
            .await?;
    if foreign_key_failures != 0 {
        bail!("backup database contains foreign-key violations");
    }
    let schema: i64 = sqlx::query_scalar("SELECT max(version) FROM schema_version")
        .fetch_one(&verify_pool)
        .await?;
    if schema != manifest.schema_version
        || schema != concord_server::db::pool::current_schema_version()
    {
        bail!("backup schema version is not supported by this binary");
    }
    let generation: String =
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(&verify_pool)
            .await?;
    if generation != manifest.database_generation {
        bail!("backup database generation does not match its manifest");
    }
    let accounts: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT user_id,credential_key_id,credential_ciphertext FROM oauth_accounts \
         WHERE provider='atproto' AND credential_state='active'",
    )
    .fetch_all(&verify_pool)
    .await?;
    for (user_id, key_id, ciphertext) in accounts {
        vault
            .decrypt(&format!("atproto:{user_id}"), &ciphertext, &key_id)
            .context("backup contains an unreadable account credential")?;
    }
    let pending: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT state_hash,credential_key_id,credential_ciphertext FROM pending_atproto_oauth \
         WHERE state='pending'",
    )
    .fetch_all(&verify_pool)
    .await?;
    for (state_hash, key_id, ciphertext) in pending {
        vault
            .decrypt(
                &format!("atproto:pending:{state_hash}"),
                &ciphertext,
                &key_id,
            )
            .context("backup contains unreadable pending OAuth state")?;
    }
    let webhooks: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id,signing_key_id,signing_ciphertext FROM webhooks \
         WHERE credential_state='active' AND signing_key_id IS NOT NULL \
           AND signing_ciphertext IS NOT NULL",
    )
    .fetch_all(&verify_pool)
    .await?;
    for (id, key_id, ciphertext) in webhooks {
        vault
            .decrypt(&format!("webhook:{id}:signing"), &ciphertext, &key_id)
            .context("backup contains an unreadable webhook signing credential")?;
    }
    let signing: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'")
            .fetch_optional(&verify_pool)
            .await?;
    if let Some(signing) = signing {
        let envelope: StoredBackupEnvelope = serde_json::from_str(&signing)
            .context("backup contains a plaintext or malformed AT Protocol signing key")?;
        vault
            .decrypt(
                "atproto:client-signing-key",
                &envelope.ciphertext,
                &envelope.key_id,
            )
            .context("backup contains an unreadable AT Protocol signing key")?;
    }
    let media: Vec<(Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT storage_key,sha256,file_size FROM attachments WHERE storage_backend='local' \
         AND media_state IN ('ready','attached')",
    )
    .fetch_all(&verify_pool)
    .await?;
    verify_pool.close().await;
    for (key, expected_hash, expected_size) in media {
        let key = key.context("ready local media lacks a storage key")?;
        let expected_hash = expected_hash.context("ready local media lacks a checksum")?;
        if expected_size < 0
            || expected_hash.len() != 64
            || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("ready local media has invalid size or checksum metadata: {key}");
        }
        let logical = format!("media/{key}");
        let path = checked_backup_path(backup, &logical)?;
        if path.metadata()?.len() != expected_size as u64 || sha256_file(&path)? != expected_hash {
            bail!("referenced media is missing or corrupt: {key}");
        }
    }
    Ok(manifest)
}

#[derive(Debug, Serialize, Deserialize)]
struct RestoreMarker {
    backup_id: String,
    phase: String,
    manifest_sha256: String,
    database_path: String,
    media_path: String,
    key_path: String,
    database_generation: Option<String>,
    operation_generation: Option<String>,
}

async fn restore_backup(
    config: &ServerConfig,
    backup: &Path,
    manifest: &BackupManifest,
) -> Result<()> {
    let database = database_path(&config.database.url)?;
    let marker_path = concord_server::operations::restore_marker_path(&config.database.url)?;
    let staged_database = staged_path(&database, &manifest.backup_id)?;
    let staged_media = staged_path(&config.storage.media_dir, &manifest.backup_id)?;
    let staged_key = staged_path(
        &config.auth.external_credentials_key_file,
        &manifest.backup_id,
    )?;
    let manifest_sha256 = sha256_file(&backup.join("manifest.json"))?;
    let bound_database = canonical_candidate(&database)?
        .to_string_lossy()
        .into_owned();
    let bound_media = canonical_candidate(&config.storage.media_dir)?
        .to_string_lossy()
        .into_owned();
    let bound_key = canonical_candidate(&config.auth.external_credentials_key_file)?
        .to_string_lossy()
        .into_owned();

    let marker = if marker_path.exists() {
        let marker: RestoreMarker = serde_json::from_slice(&fs::read(&marker_path)?)?;
        if marker.backup_id != manifest.backup_id
            || marker.manifest_sha256 != manifest_sha256
            || marker.database_path != bound_database
            || marker.media_path != bound_media
            || marker.key_path != bound_key
        {
            bail!("pending restore does not match this backup and canonical destination config");
        }
        marker
    } else {
        ensure_empty_restore_destination(
            &database,
            &config.storage.media_dir,
            &config.auth.external_credentials_key_file,
        )?;
        let marker = RestoreMarker {
            backup_id: manifest.backup_id.clone(),
            phase: "copying".into(),
            manifest_sha256: manifest_sha256.clone(),
            database_path: bound_database.clone(),
            media_path: bound_media.clone(),
            key_path: bound_key.clone(),
            database_generation: None,
            operation_generation: None,
        };
        write_restore_marker(&marker_path, &marker)?;
        restore_test_barrier("copying");
        marker
    };

    if marker.phase == "copying" {
        // No destination is activated before the prepared phase. An interrupted
        // copy is therefore safely discarded and streamed again on resume.
        ensure_empty_restore_destination(
            &database,
            &config.storage.media_dir,
            &config.auth.external_credentials_key_file,
        )?;
        remove_staging_path(&staged_database)?;
        remove_staging_path(&staged_media)?;
        remove_staging_path(&staged_key)?;
        copy_private_file(&backup.join("database.sqlite"), &staged_database)?;
        copy_tree(&backup.join("media"), &staged_media)?;
        copy_private_file(
            &backup.join("secrets/external-credentials.key"),
            &staged_key,
        )?;
        restore_test_barrier("before-rewrite");
        let (database_generation, operation_generation) =
            reconcile_staged_database(&staged_database).await?;
        restore_test_barrier("after-rewrite");
        write_restore_marker(
            &marker_path,
            &RestoreMarker {
                backup_id: manifest.backup_id.clone(),
                phase: "prepared".into(),
                manifest_sha256,
                database_path: bound_database,
                media_path: bound_media,
                key_path: bound_key,
                database_generation: Some(database_generation),
                operation_generation: Some(operation_generation),
            },
        )?;
        restore_test_barrier("prepared");
    } else if marker.phase != "prepared" {
        bail!("restore marker has an unsupported phase");
    }

    activate_staged_path(&staged_database, &database)?;
    restore_test_barrier("database-activated");
    activate_staged_tree(&staged_media, &config.storage.media_dir)?;
    activate_staged_path(&staged_key, &config.auth.external_credentials_key_file)?;
    restore_test_barrier("key-activated");
    verify_activated_restore(config, manifest, &marker_path).await?;
    fs::remove_file(&marker_path)?;
    sync_parent(&marker_path)?;
    Ok(())
}

async fn reconcile_staged_database(database: &Path) -> Result<(String, String)> {
    let url = format!("sqlite:{}?mode=rw", database.display());
    let restored = create_pool(&url).await?;
    let mut transaction = restored.begin_with("BEGIN IMMEDIATE").await?;
    let database_generation = Uuid::new_v4().to_string();
    let operation_generation = Uuid::new_v4().to_string();
    sqlx::query("UPDATE database_metadata SET generation=? WHERE singleton=1")
        .bind(&database_generation)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO operation_generations(generation,issued_at,expires_at) \
         VALUES(?,unixepoch(),unixepoch()+2592000)",
    )
    .bind(&operation_generation)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE operation_generation_state SET current_generation=? WHERE singleton=1")
        .bind(&operation_generation)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE auth_credentials SET revoked_at=unixepoch(),version=version+1 \
         WHERE kind='web_session' AND revoked_at IS NULL",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE webhook_deliveries SET state='failed',safe_error_code='restore_reconciliation_required' \
         WHERE state IN ('pending','leased')",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE atproto_publications SET status='failed', \
         safe_error_code='restore_reconciliation_required',updated_at=datetime('now') \
         WHERE status IN ('pending','update_pending','delete_pending')",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE external_jobs SET state='failed',lease_owner=NULL,lease_token=NULL,lease_until=NULL, \
         safe_error_code='restore_reconciliation_required',updated_at=datetime('now') \
         WHERE state IN ('pending','leased')",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    let checkpoint: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(&restored)
        .await?;
    if checkpoint.0 != 0 {
        bail!("restored database WAL checkpoint remained busy");
    }
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&restored)
        .await?;
    restored.close().await;
    if integrity != "ok" {
        bail!("restored database integrity check failed: {integrity}");
    }
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", database.display()));
        if sidecar.exists() {
            fs::remove_file(sidecar)?;
        }
    }
    Ok((database_generation, operation_generation))
}

async fn verify_activated_restore(
    config: &ServerConfig,
    manifest: &BackupManifest,
    marker_path: &Path,
) -> Result<()> {
    let marker: RestoreMarker = serde_json::from_slice(&fs::read(marker_path)?)?;
    if marker.phase != "prepared" {
        bail!("restore activation cannot finish from a non-prepared marker");
    }
    let expected_database_generation = marker
        .database_generation
        .as_deref()
        .context("prepared restore marker lacks database generation")?;
    let expected_operation_generation = marker
        .operation_generation
        .as_deref()
        .context("prepared restore marker lacks operation generation")?;
    let pool = create_pool(&config.database.url).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await?;
    let database_generation: String =
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(&pool)
            .await?;
    let operation: Option<(String, String, String)> = sqlx::query_as(
        "SELECT generation,typeof(issued_at),typeof(expires_at) FROM operation_generations \
         WHERE generation=(SELECT current_generation FROM operation_generation_state WHERE singleton=1)",
    )
    .fetch_optional(&pool)
    .await?;
    pool.close().await;
    if integrity != "ok"
        || database_generation != expected_database_generation
        || operation
            != Some((
                expected_operation_generation.to_owned(),
                "integer".into(),
                "integer".into(),
            ))
    {
        bail!("activated restore database does not match its prepared marker");
    }

    for file in &manifest.files {
        let destination = if let Some(relative) = file.path.strip_prefix("media/") {
            Some(config.storage.media_dir.join(relative))
        } else if file.path == "secrets/external-credentials.key" {
            Some(config.auth.external_credentials_key_file.clone())
        } else {
            None
        };
        if let Some(destination) = destination {
            let metadata = fs::symlink_metadata(&destination)?;
            if !metadata.is_file()
                || metadata.len() != file.size
                || sha256_file(&destination)? != file.sha256
            {
                bail!(
                    "activated restore component does not match manifest: {}",
                    file.path
                );
            }
        }
    }
    Ok(())
}

fn ensure_empty_restore_destination(database: &Path, media: &Path, key: &Path) -> Result<()> {
    if database.exists() {
        bail!("restore database destination must not exist");
    }
    for suffix in ["-wal", "-shm"] {
        if PathBuf::from(format!("{}{suffix}", database.display())).exists() {
            bail!("restore database sidecar destination must not exist");
        }
    }
    if media.exists() && fs::read_dir(media)?.next().is_some() {
        bail!("restore media destination must be empty");
    }
    if key.exists() {
        bail!("restore external credential key destination must not exist");
    }
    Ok(())
}

fn staged_path(destination: &Path, backup_id: &str) -> Result<PathBuf> {
    let name = destination
        .file_name()
        .context("restore destination has no file name")?
        .to_string_lossy();
    Ok(destination
        .parent()
        .context("restore destination has no parent")?
        .join(format!(".{name}.restore-{backup_id}")))
}

fn activate_staged_path(staged: &Path, destination: &Path) -> Result<()> {
    match (staged.exists(), destination.exists()) {
        (true, false) => {
            fs::rename(staged, destination)?;
            sync_parent(destination)
        }
        (false, true) => Ok(()),
        (true, true) => bail!("restore activation found both staged and destination paths"),
        (false, false) => bail!("restore activation is missing staged and destination paths"),
    }
}

fn activate_staged_tree(staged: &Path, destination: &Path) -> Result<()> {
    if !staged.exists() {
        if destination.is_dir() {
            return Ok(());
        }
        bail!("restore activation is missing staged and destination media paths");
    }
    if !destination.exists() {
        fs::rename(staged, destination)?;
        return sync_parent(destination);
    }
    if !staged.is_dir() || !destination.is_dir() {
        bail!("restore media activation encountered a non-directory path");
    }
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() && target.is_dir() {
            activate_staged_tree(&entry.path(), &target)?;
        } else if !target.exists() {
            fs::rename(entry.path(), &target)?;
            sync_parent(&target)?;
        } else {
            bail!("restore media activation found a destination collision");
        }
    }
    fs::remove_dir(staged)?;
    sync_parent(staged)
}

fn remove_staging_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn write_restore_marker(path: &Path, marker: &RestoreMarker) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    write_private_new(&temporary, &serde_json::to_vec(marker)?)?;
    fs::rename(&temporary, path)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

fn database_path(url: &str) -> Result<PathBuf> {
    let options = SqliteConnectOptions::from_str(url)?;
    let path = options.get_filename();
    if path == Path::new(":memory:") || path.as_os_str().is_empty() {
        bail!("backup/restore requires a persistent SQLite database");
    }
    Ok(path.to_owned())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let created = !destination.exists();
    fs::create_dir_all(destination)?;
    if created {
        set_private_dir(destination)?;
    }
    if !source.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_private_file(&entry.path(), &target)?;
        } else {
            bail!(
                "backup refuses non-file media entry {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn copy_private_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        let created = !parent.exists();
        fs::create_dir_all(parent)?;
        if created {
            set_private_dir(parent)?;
        }
    }
    let mut input = OpenOptions::new()
        .read(true)
        .open(source)
        .with_context(|| format!("read backup source {}", source.display()))?;
    if !input.metadata()?.is_file() {
        bail!("backup source is not a regular file: {}", source.display());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    sync_parent(destination)
}

fn reject_overlapping_destination(destination: &Path, sources: &[&Path]) -> Result<()> {
    let destination = canonical_candidate(destination)?;
    for source in sources {
        let source = canonical_candidate(source)?;
        if destination.starts_with(&source) || source.starts_with(&destination) {
            bail!(
                "backup destination overlaps source path {}",
                source.display()
            );
        }
    }
    Ok(())
}

fn canonical_candidate(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .context("path has no existing ancestor")?
                .to_owned(),
        );
        ancestor = ancestor.parent().context("path has no existing ancestor")?;
    }
    let mut normalized = ancestor.canonicalize()?;
    for component in suffix.into_iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

fn inventory_files(root: &Path) -> Result<Vec<BackupFile>> {
    fn visit(root: &Path, current: &Path, output: &mut Vec<BackupFile>) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), output)?;
            } else if kind.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative != "manifest.json" {
                    output.push(BackupFile {
                        path: relative,
                        size: entry.metadata()?.len(),
                        sha256: sha256_file(&entry.path())?,
                    });
                }
            } else {
                bail!("backup contains unsupported filesystem entry");
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

fn checked_backup_path(root: &Path, logical: &str) -> Result<PathBuf> {
    let relative = Path::new(logical);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("unsafe backup manifest path");
    }
    Ok(root.join(relative))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn set_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn create_private_destination(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path).context("create exclusive backup destination")?;
    set_private_dir(path)
}

fn write_new_key(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut random = [0_u8; 32];
    rand::rng().fill_bytes(&mut random);
    write_private_new(path, hex::encode(random).as_bytes())
}

fn backup_current_key(path: &Path, key_id: &str) -> Result<PathBuf> {
    let backup = path.with_extension(format!("previous-{key_id}"));
    let bytes = std::fs::read(path)?;
    write_private_verified(&backup, &bytes)?;
    Ok(backup)
}

fn backup_replacement_key(source: &Path, active: &Path, key_id: &str) -> Result<PathBuf> {
    let durable = active.with_extension(format!("replacement-{key_id}"));
    write_private_verified(&durable, &std::fs::read(source)?)?;
    Ok(durable)
}

fn activate_key(source: &Path, destination: &Path) -> Result<()> {
    let bytes = std::fs::read(source)?;
    let temporary = destination.with_extension(format!("activate-{}", uuid::Uuid::new_v4()));
    write_private_new(&temporary, &bytes)?;
    std::fs::rename(&temporary, destination)?;
    if let Some(parent) = destination.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

fn write_private_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Some(parent) = path.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

fn write_private_verified(path: &Path, bytes: &[u8]) -> Result<()> {
    match write_private_new(path, bytes) {
        Ok(()) => Ok(()),
        Err(error) if path.exists() => {
            let existing = std::fs::read(path)?;
            if existing == bytes {
                OpenOptions::new().read(true).open(path)?.sync_all()?;
                if let Some(parent) = path.parent() {
                    OpenOptions::new().read(true).open(parent)?.sync_all()?;
                }
                Ok(())
            } else {
                Err(error).with_context(|| {
                    format!("existing durable key copy {} differs", path.display())
                })
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "storage-fault-injection")]
fn rotation_test_barrier(stage: &str) {
    let Ok(base) = std::env::var("CONCORD_ROTATION_TEST_BARRIER") else {
        return;
    };
    let marker = PathBuf::from(format!("{base}.{stage}"));
    std::fs::write(marker, b"ready\n").expect("rotation test marker must be writable");
    loop {
        std::thread::park();
    }
}

#[cfg(not(feature = "storage-fault-injection"))]
fn rotation_test_barrier(_stage: &str) {}

#[cfg(feature = "storage-fault-injection")]
fn restore_test_barrier(stage: &str) {
    let Ok(base) = std::env::var("CONCORD_RESTORE_TEST_BARRIER") else {
        return;
    };
    if std::env::var("CONCORD_RESTORE_TEST_STAGE").as_deref() != Ok(stage) {
        return;
    }
    let marker = PathBuf::from(format!("{base}.{stage}"));
    std::fs::write(marker, b"ready\n").expect("restore test marker must be writable");
    loop {
        std::thread::park();
    }
}

#[cfg(not(feature = "storage-fault-injection"))]
fn restore_test_barrier(_stage: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_key_copy_is_idempotent_only_for_identical_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let copy = directory.path().join("key-copy");
        write_private_verified(&copy, b"first").unwrap();
        write_private_verified(&copy, b"first").unwrap();
        assert!(write_private_verified(&copy, b"different").is_err());
        assert_eq!(std::fs::read(copy).unwrap(), b"first");
    }

    #[test]
    fn backup_rejects_overlapping_paths() {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("media");
        std::fs::create_dir(&media).unwrap();
        assert!(reject_overlapping_destination(&media.join("backup"), &[media.as_path()]).is_err());
        assert!(reject_overlapping_destination(directory.path(), &[media.as_path()]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn streaming_copy_preserves_existing_parent_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let parent = directory.path().join("shared");
        std::fs::write(&source, vec![7_u8; 128 * 1024]).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o751)).unwrap();
        copy_private_file(&source, &parent.join("copy")).unwrap();
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o751
        );
        assert_eq!(
            std::fs::read(parent.join("copy")).unwrap(),
            vec![7_u8; 128 * 1024]
        );
    }

    #[test]
    fn backup_destination_is_claimed_exclusively() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("backup");
        create_private_destination(&destination).unwrap();
        std::fs::write(destination.join("owned-by-first"), b"sentinel").unwrap();
        assert!(create_private_destination(&destination).is_err());
        assert_eq!(
            std::fs::read(destination.join("owned-by-first")).unwrap(),
            b"sentinel"
        );
    }
}
