use super::{
    Digest, MediaError, Path, PathBuf, Sha256, SqlitePool, Uuid, open_media_root,
    rooted_create_dir_all, rooted_open_new, rooted_remove, rooted_rename, rooted_sync_dir,
};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Default, serde::Serialize, PartialEq, Eq)]
pub struct ImportReport {
    pub claimed: usize,
    pub imported: usize,
    pub unresolved: usize,
}

#[derive(Debug, sqlx::FromRow)]
pub(super) struct ImportItem {
    pub(super) attachment_id: String,
    pub(super) previous_url: String,
    pub(super) expected_size: Option<i64>,
    pub(super) claim_token: String,
}

/// Imports a bounded batch of historical external attachments. Each row is
/// fenced in the ledger before network I/O and the attachment locator changes
/// only after the downloaded bytes are durably stored and verified.
pub async fn import_legacy_batch(
    pool: &SqlitePool,
    root: &Path,
    client: &crate::egress::ControlledHttpClient,
    max_bytes: u64,
    lease_seconds: i64,
    limit: i64,
) -> Result<ImportReport, MediaError> {
    if max_bytes == 0 || !(1..=3600).contains(&lease_seconds) || limit != 1 {
        return Err(MediaError::Invalid);
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT attachment_id FROM media_import_ledger WHERE outcome='pending' OR (outcome='importing' AND claim_until<datetime('now')) ORDER BY attachment_id LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await?;
    let mut items = Vec::new();
    for id in ids {
        let token = Uuid::new_v4().to_string();
        let row = sqlx::query_as::<_, ImportItem>(
            "UPDATE media_import_ledger SET outcome='importing',claim_token=?,claim_until=datetime('now',?),attempted_at=datetime('now'),detail_code=NULL WHERE attachment_id=? AND (outcome='pending' OR (outcome='importing' AND claim_until<datetime('now'))) RETURNING attachment_id,previous_url,expected_size,claim_token",
        )
        .bind(&token)
        .bind(format!("+{lease_seconds} seconds"))
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(row) = row {
            items.push(row);
        }
    }
    transaction.commit().await?;

    let mut report = ImportReport {
        claimed: items.len(),
        ..ImportReport::default()
    };
    for item in items {
        match import_one(pool, root, client, max_bytes, lease_seconds, &item).await {
            Ok(true) => report.imported += 1,
            Ok(false) => report.unresolved += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

pub(super) async fn import_one(
    pool: &SqlitePool,
    root: &Path,
    client: &crate::egress::ControlledHttpClient,
    max_bytes: u64,
    lease_seconds: i64,
    item: &ImportItem,
) -> Result<bool, MediaError> {
    let url = match reqwest::Url::parse(&item.previous_url) {
        Ok(url) => url,
        Err(_) => {
            record_import_failure(pool, item, "download_failed", "invalid_url").await?;
            return Ok(false);
        }
    };
    let request = match client.request(
        reqwest::Method::GET,
        url,
        crate::egress::RedirectPolicy::FollowSafeGet,
    ) {
        Ok(request) => request,
        Err(_) => {
            record_import_failure(pool, item, "download_failed", "destination_denied").await?;
            return Ok(false);
        }
    };
    let mut response = match client.send_streaming(request).await {
        Ok(response) if response.status.is_success() => response,
        Ok(_) => {
            record_import_failure(pool, item, "download_failed", "http_status").await?;
            return Ok(false);
        }
        Err(_) => {
            record_import_failure(pool, item, "download_failed", "transport").await?;
            return Ok(false);
        }
    };
    let object_seed = format!("{}:{}", item.attachment_id, item.claim_token);
    let object_name = hex::encode(Sha256::digest(object_seed.as_bytes()));
    let shard = &object_name[..2];
    let storage_key = format!("objects/{shard}/{object_name}");
    let rooted = open_media_root(root)?;
    let object_dir = PathBuf::from("objects").join(shard);
    let staging_dir = PathBuf::from("staging");
    rooted_create_dir_all(rooted.clone(), object_dir.clone()).await?;
    rooted_create_dir_all(rooted.clone(), staging_dir.clone()).await?;
    let staging = staging_dir.join(format!("import-{}.part", item.claim_token));
    let final_path = PathBuf::from(&storage_key);
    let mut file = rooted_open_new(rooted.clone(), staging.clone()).await?;
    let mut hasher = Sha256::new();
    let mut actual = 0_u64;
    let renewal_interval =
        std::time::Duration::from_secs_f64((lease_seconds as f64 / 3.0).clamp(0.1, 30.0));
    let mut last_renewal = tokio::time::Instant::now();
    loop {
        let chunk = match response.next_chunk().await {
            Ok(chunk) => chunk,
            Err(_) => {
                drop(file);
                let _ = rooted_remove(rooted.clone(), staging.clone()).await;
                record_import_failure(pool, item, "download_failed", "transport").await?;
                return Ok(false);
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        actual = match actual.checked_add(chunk.len() as u64) {
            Some(value) if value <= max_bytes => value,
            _ => {
                drop(file);
                let _ = rooted_remove(rooted.clone(), staging.clone()).await;
                record_import_failure(pool, item, "download_failed", "invalid_size").await?;
                return Ok(false);
            }
        };
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        if last_renewal.elapsed() >= renewal_interval {
            let renewed=sqlx::query("UPDATE media_import_ledger SET claim_until=datetime('now',?) WHERE attachment_id=? AND outcome='importing' AND claim_token=?")
                .bind(format!("+{lease_seconds} seconds")).bind(&item.attachment_id).bind(&item.claim_token).execute(pool).await?;
            if renewed.rows_affected() != 1 {
                drop(file);
                let _ = rooted_remove(rooted.clone(), staging.clone()).await;
                return Ok(false);
            }
            last_renewal = tokio::time::Instant::now();
        }
    }
    if actual == 0 {
        drop(file);
        let _ = rooted_remove(rooted.clone(), staging.clone()).await;
        record_import_failure(pool, item, "download_failed", "invalid_size").await?;
        return Ok(false);
    }
    if item
        .expected_size
        .is_some_and(|expected| expected < 0 || expected as u64 != actual)
    {
        drop(file);
        let _ = rooted_remove(rooted.clone(), staging.clone()).await;
        let changed=sqlx::query("UPDATE media_import_ledger SET outcome='size_mismatch',actual_size=?,detail_code='expected_size_mismatch',claim_token=NULL,claim_until=NULL,completed_at=datetime('now') WHERE attachment_id=? AND outcome='importing' AND claim_token=?")
            .bind(actual as i64).bind(&item.attachment_id).bind(&item.claim_token).execute(pool).await?;
        let _ = changed;
        return Ok(false);
    }
    let checksum = hex::encode(hasher.finalize());
    let renewed=sqlx::query("UPDATE media_import_ledger SET claim_until=datetime('now',?) WHERE attachment_id=? AND outcome='importing' AND claim_token=?")
        .bind(format!("+{lease_seconds} seconds")).bind(&item.attachment_id).bind(&item.claim_token).execute(pool).await?;
    if renewed.rows_affected() != 1 {
        drop(file);
        let _ = rooted_remove(rooted.clone(), staging.clone()).await;
        return Ok(false);
    }
    file.sync_all().await?;
    drop(file);
    rooted_rename(rooted.clone(), staging, final_path).await?;
    rooted_sync_dir(rooted.clone(), object_dir).await?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed=sqlx::query("UPDATE media_import_ledger SET outcome='imported',actual_size=?,sha256=?,detail_code=NULL,claim_token=NULL,claim_until=NULL,completed_at=datetime('now') WHERE attachment_id=? AND outcome='importing' AND claim_token=?")
        .bind(actual as i64).bind(&checksum).bind(&item.attachment_id).bind(&item.claim_token).execute(&mut *transaction).await?;
    if changed.rows_affected() != 1 {
        transaction.rollback().await?;
        let _ = rooted_remove(rooted.clone(), PathBuf::from(&storage_key)).await;
        return Ok(false);
    }
    let updated=sqlx::query("UPDATE attachments SET storage_backend='local',storage_key=?,file_size=?,sha256=?,reserved_bytes=0,ready_at=datetime('now'),media_state=CASE WHEN message_id IS NULL THEN 'ready' ELSE 'attached' END,state_version=state_version+1 WHERE id=? AND media_state='legacy_external'")
        .bind(&storage_key).bind(actual as i64).bind(&checksum).bind(&item.attachment_id).execute(&mut *transaction).await?;
    if updated.rows_affected() != 1 {
        transaction.rollback().await?;
        let _ = rooted_remove(rooted, PathBuf::from(&storage_key)).await;
        return Ok(false);
    }
    transaction.commit().await?;
    Ok(true)
}

pub(super) async fn record_import_failure(
    pool: &SqlitePool,
    item: &ImportItem,
    outcome: &str,
    code: &str,
) -> Result<(), MediaError> {
    sqlx::query("UPDATE media_import_ledger SET outcome=?,detail_code=?,claim_token=NULL,claim_until=NULL,completed_at=datetime('now') WHERE attachment_id=? AND outcome='importing' AND claim_token=?")
        .bind(outcome).bind(code).bind(&item.attachment_id).bind(&item.claim_token).execute(pool).await?;
    Ok(())
}

pub async fn retry_legacy_import(
    pool: &SqlitePool,
    attachment_id: &str,
) -> Result<bool, MediaError> {
    Ok(sqlx::query("UPDATE media_import_ledger SET outcome='pending',detail_code=NULL,claim_token=NULL,claim_until=NULL,completed_at=NULL WHERE attachment_id=? AND outcome IN ('download_failed','size_mismatch','missing_credentials','missing_data','ambiguous_reference')")
        .bind(attachment_id).execute(pool).await?.rows_affected()==1)
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ExternalReferenceInventory {
    pub attachment_id: String,
    pub previous_url: String,
    pub previous_cid: Option<String>,
    pub record_uri: Option<String>,
    pub reference_outcome: Option<String>,
    pub outcome: String,
    pub detail_code: Option<String>,
    pub previously_public: bool,
}

pub async fn external_reference_inventory(
    pool: &SqlitePool,
) -> Result<Vec<ExternalReferenceInventory>, MediaError> {
    Ok(sqlx::query_as("SELECT l.attachment_id,l.previous_url,l.previous_cid,l.record_uri,l.reference_outcome,l.outcome,l.detail_code,a.previously_public FROM media_import_ledger l JOIN attachments a ON a.id=l.attachment_id ORDER BY l.attachment_id")
        .fetch_all(pool).await?)
}
