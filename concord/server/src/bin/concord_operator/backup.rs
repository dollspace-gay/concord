use super::{
    Deserialize, Path, Result, SecretVault, Serialize, ServerConfig, SqliteConnectOptions, Uuid,
    bail, checked_backup_path, copy_private_file, copy_tree, create_private_destination,
    database_path, inventory_files, migration_preflight, reject_overlapping_destination,
    sha256_file, write_private_new,
};
use anyhow::Context;
use std::fs;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct BackupManifest {
    pub(super) format_version: u32,
    pub(super) backup_id: String,
    pub(super) created_at: String,
    pub(super) schema_version: i64,
    pub(super) database_generation: String,
    pub(super) files: Vec<BackupFile>,
    pub(super) external_credentials_key_id: String,
    pub(super) has_jwt_secret_file: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct BackupFile {
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) sha256: String,
}

#[derive(Deserialize)]
pub(super) struct StoredBackupEnvelope {
    pub(super) key_id: String,
    pub(super) ciphertext: String,
}

pub(super) async fn create_backup(
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

pub(super) async fn verify_backup(backup: &Path) -> Result<BackupManifest> {
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
