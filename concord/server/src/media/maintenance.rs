use super::{
    Digest, MAINTENANCE_BATCH_SIZE, MediaError, Path, PathBuf, Sha256, SqlitePool, Uuid,
    open_media_root, open_rooted_media, rooted_remove,
};
use tokio::io::AsyncReadExt;

/// Removes expired staging files and rows left by interrupted requests.
pub async fn collect_expired(
    pool: &SqlitePool,
    root: &Path,
    grace_seconds: i64,
) -> Result<u64, MediaError> {
    let rooted = open_media_root(root)?;
    let rows:Vec<(String,String)>=sqlx::query_as("SELECT id,storage_key FROM attachments WHERE media_state='staging' AND upload_updated_at < datetime('now', ?) ORDER BY upload_updated_at,id LIMIT ?")
        .bind(format!("-{grace_seconds} seconds")).bind(MAINTENANCE_BATCH_SIZE).fetch_all(pool).await?;
    let mut count = 0;
    for (id, key) in rows {
        let claimed=sqlx::query("UPDATE attachments SET media_state='failed',reserved_bytes=0,state_version=state_version+1 WHERE id=? AND media_state='staging' AND upload_updated_at < datetime('now', ?)")
            .bind(&id).bind(format!("-{grace_seconds} seconds")).execute(pool).await?;
        if claimed.rows_affected() != 1 {
            continue;
        }
        let _ = rooted_remove(
            rooted.clone(),
            PathBuf::from("staging").join(format!("{id}.part")),
        )
        .await;
        if safe_storage_key(&key) {
            let _ = rooted_remove(rooted.clone(), PathBuf::from(key)).await;
        }
        count += 1;
    }
    Ok(count)
}

pub async fn collect_deleted(pool: &SqlitePool, root: &Path) -> Result<u64, MediaError> {
    let rooted = open_media_root(root)?;
    let rows:Vec<(String,String)>=sqlx::query_as(
        "SELECT id,storage_key FROM attachments WHERE media_state='deleting' AND delete_after<=datetime('now') ORDER BY delete_after,id LIMIT ?",
    ).bind(MAINTENANCE_BATCH_SIZE).fetch_all(pool).await?;
    let mut count = 0;
    for (id, key) in rows {
        if !safe_storage_key(&key) {
            continue;
        }
        match rooted_remove(rooted.clone(), PathBuf::from(&key)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        count += sqlx::query("UPDATE attachments SET media_state='deleted',state_version=state_version+1,storage_key=NULL WHERE id=? AND media_state='deleting'")
            .bind(id).execute(pool).await?.rows_affected();
    }
    Ok(count)
}

pub async fn reconcile_interrupted(pool: &SqlitePool, root: &Path) -> Result<u64, MediaError> {
    let rooted = open_media_root(root)?;
    let rows:Vec<(String,String,i64)>=sqlx::query_as(
        "SELECT id,storage_key,reserved_bytes FROM attachments WHERE media_state='staging' AND storage_backend='local' ORDER BY upload_updated_at,id LIMIT ?",
    ).bind(MAINTENANCE_BATCH_SIZE).fetch_all(pool).await?;
    let mut recovered = 0;
    for (id, key, reserved) in rows {
        if !safe_storage_key(&key) {
            continue;
        }
        let Ok(mut file) = open_rooted_media(root, &key).await else {
            continue;
        };
        let metadata = file.metadata().await?;
        if !metadata.is_file() || metadata.len() > reserved.max(0) as u64 {
            let _ = rooted_remove(rooted.clone(), PathBuf::from(&key)).await;
            sqlx::query("UPDATE attachments SET media_state='failed',reserved_bytes=0,state_version=state_version+1 WHERE id=? AND media_state='staging'")
                .bind(&id).execute(pool).await?;
            continue;
        }
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let changed=sqlx::query("UPDATE attachments SET media_state='ready',file_size=?,sha256=?,reserved_bytes=0,ready_at=datetime('now'),state_version=state_version+1 WHERE id=? AND media_state='staging'")
            .bind(metadata.len() as i64).bind(hex::encode(hasher.finalize())).bind(&id).execute(pool).await?;
        recovered += changed.rows_affected();
    }
    Ok(recovered)
}

pub(crate) fn safe_storage_key(key: &str) -> bool {
    let path = Path::new(key);
    !path.is_absolute()
        && key.starts_with("objects/")
        && path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

pub(crate) fn local_attachment_id(url: &str) -> Option<&str> {
    let id = url.strip_prefix("/api/uploads/")?;
    (!id.is_empty() && !id.contains('/') && Uuid::parse_str(id).is_ok()).then_some(id)
}
