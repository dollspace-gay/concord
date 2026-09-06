use std::fs::OpenOptions;

use std::io::{Read, Write};

use std::path::{Path, PathBuf};

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

#[cfg(test)]
#[path = "concord_operator/tests.rs"]
mod tests;

#[path = "concord_operator/administration.rs"]
mod administration;
#[path = "concord_operator/backup.rs"]
mod backup;
#[path = "concord_operator/backup_files.rs"]
mod backup_files;
#[path = "concord_operator/cli.rs"]
mod cli;
#[path = "concord_operator/external_jobs.rs"]
mod external_jobs;
#[path = "concord_operator/keys.rs"]
mod keys;
#[path = "concord_operator/restore.rs"]
mod restore;
#[path = "concord_operator/restore_files.rs"]
mod restore_files;
#[path = "concord_operator/test_barriers.rs"]
mod test_barriers;
use administration::insert_operator_audit;
use administration::print_admin_inventory;
use administration::recover_admin;

use administration::revoke_all_user_credentials;
use administration::transfer_admin;
use administration::validate_operator_reason;
use backup::BackupFile;
use backup::BackupManifest;

use backup::create_backup;
use backup::verify_backup;
use backup_files::canonical_candidate;
use backup_files::checked_backup_path;
use backup_files::copy_private_file;
use backup_files::copy_tree;
use backup_files::database_path;
use backup_files::inventory_files;
use backup_files::reject_overlapping_destination;
use backup_files::sha256_file;
use cli::Cli;
use cli::Command;
use external_jobs::print_external_jobs;
use external_jobs::reconcile_atproto_publication;
use external_jobs::retry_external_job;
use keys::activate_key;
use keys::backup_current_key;
use keys::backup_replacement_key;
use keys::create_private_destination;
use keys::set_private_dir;
use keys::write_new_key;
use keys::write_private_new;

use restore::RestoreMarker;

use restore::restore_backup;

use restore_files::activate_staged_path;
use restore_files::activate_staged_tree;
use restore_files::ensure_empty_restore_destination;
use restore_files::remove_staging_path;
use restore_files::staged_path;
use restore_files::sync_parent;
use restore_files::write_restore_marker;
#[cfg(feature = "storage-fault-injection")]
use test_barriers::restore_test_barrier;
#[cfg(not(feature = "storage-fault-injection"))]
use test_barriers::restore_test_barrier;
#[cfg(feature = "storage-fault-injection")]
use test_barriers::rotation_test_barrier;
#[cfg(not(feature = "storage-fault-injection"))]
use test_barriers::rotation_test_barrier;
