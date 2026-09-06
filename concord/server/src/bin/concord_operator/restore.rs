use super::{
    BackupManifest, Deserialize, Path, PathBuf, Result, Serialize, ServerConfig, Uuid,
    activate_staged_path, activate_staged_tree, bail, canonical_candidate, copy_private_file,
    copy_tree, create_pool, database_path, ensure_empty_restore_destination, remove_staging_path,
    restore_test_barrier, sha256_file, staged_path, sync_parent, write_restore_marker,
};
use anyhow::Context;
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct RestoreMarker {
    pub(super) backup_id: String,
    pub(super) phase: String,
    pub(super) manifest_sha256: String,
    pub(super) database_path: String,
    pub(super) media_path: String,
    pub(super) key_path: String,
    pub(super) database_generation: Option<String>,
    pub(super) operation_generation: Option<String>,
}

pub(super) async fn restore_backup(
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

pub(super) async fn reconcile_staged_database(database: &Path) -> Result<(String, String)> {
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

pub(super) async fn verify_activated_restore(
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
