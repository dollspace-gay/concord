//! Instance-owned private attachment storage.
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const MAINTENANCE_BATCH_SIZE: i64 = 100;
const UPLOAD_PROGRESS_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

fn open_media_root(path: &Path) -> Result<Arc<cap_std::fs::Dir>, MediaError> {
    Ok(Arc::new(cap_std::fs::Dir::open_ambient_dir(
        path,
        cap_std::ambient_authority(),
    )?))
}
async fn rooted_create_dir_all(
    root: Arc<cap_std::fs::Dir>,
    path: PathBuf,
) -> Result<(), MediaError> {
    tokio::task::spawn_blocking(move || root.create_dir_all(path))
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(())
}
async fn rooted_open_new(
    root: Arc<cap_std::fs::Dir>,
    path: PathBuf,
) -> Result<tokio::fs::File, MediaError> {
    let file = tokio::task::spawn_blocking(move || {
        root.open_with(
            path,
            cap_std::fs::OpenOptions::new().create_new(true).write(true),
        )
    })
    .await
    .map_err(|_| MediaError::Invalid)??;
    Ok(tokio::fs::File::from_std(file.into_std()))
}
pub async fn open_rooted_media(path: &Path, key: &str) -> Result<tokio::fs::File, MediaError> {
    if !safe_storage_key(key) {
        return Err(MediaError::Invalid);
    }
    let root = open_media_root(path)?;
    let key = PathBuf::from(key);
    let file = tokio::task::spawn_blocking(move || root.open(key))
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(tokio::fs::File::from_std(file.into_std()))
}
async fn rooted_remove(root: Arc<cap_std::fs::Dir>, path: PathBuf) -> Result<(), std::io::Error> {
    tokio::task::spawn_blocking(move || root.remove_file(path))
        .await
        .map_err(|_| std::io::Error::other("media filesystem task failed"))?
}
async fn rooted_rename(
    root: Arc<cap_std::fs::Dir>,
    from: PathBuf,
    to: PathBuf,
) -> Result<(), MediaError> {
    let target = root.clone();
    tokio::task::spawn_blocking(move || root.rename(from, &target, to))
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(())
}
async fn rooted_sync_dir(root: Arc<cap_std::fs::Dir>, path: PathBuf) -> Result<(), MediaError> {
    tokio::task::spawn_blocking(move || root.open(path)?.into_std().sync_all())
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media request is invalid")]
    Invalid,
    #[error("media quota exceeded")]
    TooLarge,
    #[error("media storage operation failed")]
    Storage(#[source] std::io::Error),
    #[error("media database operation failed")]
    Database(#[source] sqlx::Error),
}
impl From<std::io::Error> for MediaError {
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e)
    }
}
impl From<sqlx::Error> for MediaError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

pub struct MediaUpload {
    pub id: String,
    pool: SqlitePool,
    file: tokio::fs::File,
    staging_path: PathBuf,
    final_path: PathBuf,
    root: Arc<cap_std::fs::Dir>,
    storage_key: String,
    hasher: Sha256,
    written: u64,
    max_bytes: u64,
    last_progress_update: tokio::time::Instant,
    metric: crate::runtime_metrics::Timer,
}
pub struct StartMedia<'a> {
    pub owner_id: &'a str,
    pub intent: MediaIntent,
    pub original_filename: &'a str,
    pub content_type: &'a str,
    pub max_bytes: u64,
    pub per_user_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaIntent {
    Message {
        conversation_id: String,
    },
    ServerAsset {
        server_id: String,
        purpose: ServerMediaPurpose,
    },
    UserAsset {
        purpose: UserMediaPurpose,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerMediaPurpose {
    Emoji,
    Sticker,
    Avatar,
    MemberAvatar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserMediaPurpose {
    Avatar,
    Banner,
}

impl MediaIntent {
    fn columns<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> (
        &'static str,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
    ) {
        match self {
            Self::Message { conversation_id } => {
                ("message", Some(conversation_id.as_str()), None, None)
            }
            Self::ServerAsset { server_id, purpose } => (
                match purpose {
                    ServerMediaPurpose::Emoji => "emoji",
                    ServerMediaPurpose::Sticker => "sticker",
                    ServerMediaPurpose::Avatar => "server_avatar",
                    ServerMediaPurpose::MemberAvatar => "server_member_avatar",
                },
                None,
                Some(server_id.as_str()),
                matches!(purpose, ServerMediaPurpose::MemberAvatar).then_some(owner_id),
            ),
            Self::UserAsset { purpose } => (
                match purpose {
                    UserMediaPurpose::Avatar => "user_avatar",
                    UserMediaPurpose::Banner => "user_banner",
                },
                None,
                None,
                Some(owner_id),
            ),
        }
    }
}
impl MediaUpload {
    pub async fn start(
        pool: SqlitePool,
        root: &Path,
        request: StartMedia<'_>,
    ) -> Result<Self, MediaError> {
        Self::start_inner(
            pool,
            root,
            request,
            #[cfg(test)]
            None,
        )
        .await
    }

    async fn start_inner(
        pool: SqlitePool,
        root: &Path,
        request: StartMedia<'_>,
        #[cfg(test)] post_reserve_barrier: Option<(
            Arc<tokio::sync::Notify>,
            Arc<tokio::sync::Notify>,
        )>,
    ) -> Result<Self, MediaError> {
        let metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Upload);
        let StartMedia {
            owner_id,
            intent,
            original_filename,
            content_type,
            max_bytes,
            per_user_bytes,
            total_bytes,
        } = request;
        if max_bytes == 0
            || per_user_bytes < max_bytes
            || total_bytes < per_user_bytes
            || original_filename.is_empty()
            || content_type.is_empty()
        {
            return Err(MediaError::Invalid);
        }
        let id = Uuid::new_v4().to_string();
        let shard = &id[..2];
        let rooted = open_media_root(root)?;
        let staging_dir = PathBuf::from("staging");
        let objects_dir = PathBuf::from("objects").join(shard);
        rooted_create_dir_all(rooted.clone(), staging_dir.clone()).await?;
        rooted_create_dir_all(rooted.clone(), objects_dir).await?;
        rooted_sync_dir(rooted.clone(), PathBuf::from(".")).await?;
        rooted_sync_dir(rooted.clone(), PathBuf::from("objects")).await?;
        let staging_path = staging_dir.join(format!("{id}.part"));
        let storage_key = format!("objects/{shard}/{id}");
        let final_path = PathBuf::from(&storage_key);
        reserve(
            &pool,
            Reservation {
                id: &id,
                owner: owner_id,
                intent,
                filename: original_filename,
                content_type,
                storage_key: &storage_key,
                reserve: max_bytes,
                per_user: per_user_bytes,
                total: total_bytes,
            },
        )
        .await?;
        #[cfg(test)]
        if let Some((reserved, resume)) = post_reserve_barrier {
            reserved.notify_one();
            resume.notified().await;
        }
        let file = match rooted_open_new(rooted.clone(), staging_path.clone()).await {
            Ok(file) => file,
            Err(error) => {
                let _ = sqlx::query("UPDATE attachments SET media_state='failed',reserved_bytes=0,state_version=state_version+1 WHERE id=? AND media_state='staging'")
                    .bind(&id)
                    .execute(&pool)
                    .await;
                return Err(error);
            }
        };
        Ok(Self {
            id,
            pool,
            file,
            staging_path,
            final_path,
            root: rooted,
            storage_key,
            hasher: Sha256::new(),
            written: 0,
            max_bytes,
            last_progress_update: tokio::time::Instant::now(),
            metric,
        })
    }
    pub async fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), MediaError> {
        self.written = self
            .written
            .checked_add(chunk.len() as u64)
            .ok_or(MediaError::TooLarge)?;
        if self.written > self.max_bytes {
            return Err(MediaError::TooLarge);
        }
        self.file.write_all(chunk).await?;
        self.hasher.update(chunk);
        if self.last_progress_update.elapsed() >= UPLOAD_PROGRESS_UPDATE_INTERVAL {
            let updated = sqlx::query("UPDATE attachments SET upload_updated_at=datetime('now') WHERE id=? AND media_state='staging'")
                .bind(&self.id)
                .execute(&self.pool)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(MediaError::Invalid);
            }
            self.last_progress_update = tokio::time::Instant::now();
        }
        Ok(())
    }
    pub async fn finish(mut self) -> Result<ReadyMedia, MediaError> {
        if self.written == 0 {
            self.abort().await;
            return Err(MediaError::Invalid);
        }
        let claimed=sqlx::query("UPDATE attachments SET upload_updated_at=datetime('now'),state_version=state_version+1 WHERE id=? AND media_state='staging'")
            .bind(&self.id).execute(&self.pool).await?;
        if claimed.rows_affected() != 1 {
            self.abort().await;
            return Err(MediaError::Invalid);
        }
        self.file.flush().await?;
        self.file.sync_all().await?;
        rooted_rename(
            self.root.clone(),
            self.staging_path.clone(),
            self.final_path.clone(),
        )
        .await?;
        rooted_sync_dir(
            self.root.clone(),
            self.final_path
                .parent()
                .ok_or(MediaError::Invalid)?
                .to_path_buf(),
        )
        .await?;
        let checksum = hex::encode(self.hasher.finalize());
        let updated=sqlx::query("UPDATE attachments SET media_state='ready',state_version=state_version+1,file_size=?,sha256=?,reserved_bytes=0,ready_at=datetime('now') WHERE id=? AND media_state='staging'")
            .bind(self.written as i64).bind(&checksum).bind(&self.id).execute(&self.pool).await?;
        if updated.rows_affected() != 1 {
            let _ = rooted_remove(self.root.clone(), self.final_path.clone()).await;
            return Err(MediaError::Invalid);
        }
        self.metric.succeed();
        Ok(ReadyMedia {
            id: self.id,
            file_size: self.written,
            sha256: checksum,
            storage_key: self.storage_key,
        })
    }
    pub async fn abort(self) {
        let _ = rooted_remove(self.root.clone(), self.staging_path.clone()).await;
        let _ = rooted_remove(self.root.clone(), self.final_path.clone()).await;
        let _=sqlx::query("UPDATE attachments SET media_state='failed',state_version=state_version+1,reserved_bytes=0 WHERE id=? AND media_state='staging'").bind(&self.id).execute(&self.pool).await;
    }
}

struct Reservation<'a> {
    id: &'a str,
    owner: &'a str,
    intent: MediaIntent,
    filename: &'a str,
    content_type: &'a str,
    storage_key: &'a str,
    reserve: u64,
    per_user: u64,
    total: u64,
}
async fn reserve(pool: &SqlitePool, request: Reservation<'_>) -> Result<(), MediaError> {
    let Reservation {
        id,
        owner,
        intent,
        filename,
        content_type,
        storage_key,
        reserve,
        per_user,
        total,
    } = request;
    let reserve = i64::try_from(reserve).map_err(|_| MediaError::TooLarge)?;
    let per_user = i64::try_from(per_user).map_err(|_| MediaError::TooLarge)?;
    let total = i64::try_from(total).map_err(|_| MediaError::TooLarge)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let user_used:i64=sqlx::query_scalar("SELECT COALESCE(SUM(CASE WHEN media_state='staging' THEN reserved_bytes ELSE file_size END),0) FROM attachments WHERE uploader_id=? AND media_state IN ('staging','ready','attached')")
        .bind(owner).fetch_one(&mut *transaction).await?;
    let total_used:i64=sqlx::query_scalar("SELECT COALESCE(SUM(CASE WHEN media_state='staging' THEN reserved_bytes ELSE file_size END),0) FROM attachments WHERE media_state IN ('staging','ready','attached')")
        .fetch_one(&mut *transaction).await?;
    if user_used.saturating_add(reserve) > per_user || total_used.saturating_add(reserve) > total {
        return Err(MediaError::TooLarge);
    }
    let (purpose, conversation, managed_server, managed_user) = intent.columns(owner);
    sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,media_purpose,managed_server_id,managed_user_id,filename,original_filename,content_type,file_size,media_state,storage_backend,storage_key,reserved_bytes,upload_updated_at) VALUES(?,?,?,?,?,?,?,?,?,0,'staging','local',?,?,datetime('now'))")
        .bind(id).bind(owner).bind(conversation).bind(purpose).bind(managed_server).bind(managed_user).bind(id).bind(filename).bind(content_type)
        .bind(storage_key).bind(reserve).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(())
}

pub struct ReadyMedia {
    pub id: String,
    pub file_size: u64,
    pub sha256: String,
    pub storage_key: String,
}

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

#[derive(Debug, Default, serde::Serialize, PartialEq, Eq)]
pub struct ImportReport {
    pub claimed: usize,
    pub imported: usize,
    pub unresolved: usize,
}

#[derive(Debug, sqlx::FromRow)]
struct ImportItem {
    attachment_id: String,
    previous_url: String,
    expected_size: Option<i64>,
    claim_token: String,
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

async fn import_one(
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

async fn record_import_failure(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use tokio::net::TcpListener;

    async fn http_fixture(body: Vec<u8>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(&body).await.unwrap();
        });
        (address, task)
    }
    async fn fixture() -> (tempfile::TempDir, SqlitePool, String) {
        let d = tempfile::tempdir().unwrap();
        let p = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&p).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('u','u')")
            .execute(&p)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('s','s','u')")
            .execute(&p)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('c','s','#c')")
            .execute(&p)
            .await
            .unwrap();
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='c'")
                .fetch_one(&p)
                .await
                .unwrap();
        (d, p, conversation)
    }
    #[tokio::test]
    async fn durable_ready_transition_and_checksum() {
        let (d, p, conversation) = fixture().await;
        let mut u = MediaUpload::start(
            p.clone(),
            d.path(),
            StartMedia {
                owner_id: "u",
                intent: MediaIntent::Message {
                    conversation_id: conversation.clone(),
                },
                original_filename: "x.txt",
                content_type: "text/plain",
                max_bytes: 8,
                per_user_bytes: 16,
                total_bytes: 64,
            },
        )
        .await
        .unwrap();
        u.write_chunk(b"hello").await.unwrap();
        let ready = u.finish().await.unwrap();
        assert_eq!(ready.file_size, 5);
        assert_eq!(
            tokio::fs::read(d.path().join(ready.storage_key))
                .await
                .unwrap(),
            b"hello"
        );
        let state: (String, String) =
            sqlx::query_as("SELECT media_state,sha256 FROM attachments WHERE id=?")
                .bind(ready.id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(state.0, "ready");
        assert_eq!(state.1, ready.sha256);
    }
    #[tokio::test]
    async fn oversize_upload_is_failed_on_abort() {
        let before = crate::runtime_metrics::snapshot();
        let upload_index = crate::runtime_metrics::Operation::Upload as usize;
        let (d, p, conversation) = fixture().await;
        let mut u = MediaUpload::start(
            p.clone(),
            d.path(),
            StartMedia {
                owner_id: "u",
                intent: MediaIntent::Message {
                    conversation_id: conversation.clone(),
                },
                original_filename: "x",
                content_type: "text/plain",
                max_bytes: 3,
                per_user_bytes: 16,
                total_bytes: 64,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            u.write_chunk(b"four").await,
            Err(MediaError::TooLarge)
        ));
        let id = u.id.clone();
        u.abort().await;
        let state: String = sqlx::query_scalar("SELECT media_state FROM attachments WHERE id=?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(state, "failed");
        let after = crate::runtime_metrics::snapshot();
        assert!(after.failed[upload_index] > before.failed[upload_index]);
    }

    #[tokio::test]
    async fn cancelled_start_after_reservation_leaves_only_collectable_state() {
        let (d, p, conversation) = fixture().await;
        let reserved = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let task = {
            let pool = p.clone();
            let root = d.path().to_owned();
            let reserved = reserved.clone();
            let resume = resume.clone();
            tokio::spawn(async move {
                MediaUpload::start_inner(
                    pool,
                    &root,
                    StartMedia {
                        owner_id: "u",
                        intent: MediaIntent::Message {
                            conversation_id: conversation,
                        },
                        original_filename: "cancelled.bin",
                        content_type: "application/octet-stream",
                        max_bytes: 8,
                        per_user_bytes: 16,
                        total_bytes: 64,
                    },
                    Some((reserved, resume)),
                )
                .await
            })
        };
        reserved.notified().await;
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        let row: (String, i64) = sqlx::query_as(
            "SELECT media_state,reserved_bytes FROM attachments WHERE original_filename='cancelled.bin'",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(row, ("staging".into(), 8));
        assert_eq!(
            std::fs::read_dir(d.path().join("staging")).unwrap().count(),
            0
        );
        sqlx::query("UPDATE attachments SET upload_updated_at=datetime('now','-10 seconds') WHERE original_filename='cancelled.bin'")
            .execute(&p)
            .await
            .unwrap();
        assert_eq!(collect_expired(&p, d.path(), 1).await.unwrap(), 1);
        let row: (String, i64) = sqlx::query_as(
            "SELECT media_state,reserved_bytes FROM attachments WHERE original_filename='cancelled.bin'",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(row, ("failed".into(), 0));
    }

    #[tokio::test]
    async fn fragmented_upload_does_not_write_progress_for_every_chunk() {
        let (d, p, conversation) = fixture().await;
        sqlx::query("CREATE TABLE progress_updates(count INTEGER NOT NULL)")
            .execute(&p)
            .await
            .unwrap();
        sqlx::query("INSERT INTO progress_updates(count) VALUES(0)")
            .execute(&p)
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER count_upload_progress AFTER UPDATE OF upload_updated_at ON attachments BEGIN UPDATE progress_updates SET count=count+1; END")
            .execute(&p)
            .await
            .unwrap();
        let mut upload = MediaUpload::start(
            p.clone(),
            d.path(),
            StartMedia {
                owner_id: "u",
                intent: MediaIntent::Message {
                    conversation_id: conversation,
                },
                original_filename: "fragments.bin",
                content_type: "application/octet-stream",
                max_bytes: 2_000,
                per_user_bytes: 4_000,
                total_bytes: 8_000,
            },
        )
        .await
        .unwrap();
        for _ in 0..1_000 {
            upload.write_chunk(b"x").await.unwrap();
        }
        let updates: i64 = sqlx::query_scalar("SELECT count FROM progress_updates")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(updates, 0);
        upload.finish().await.unwrap();
        let updates: i64 = sqlx::query_scalar("SELECT count FROM progress_updates")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(
            updates, 1,
            "only the final fencing update should touch progress"
        );
    }

    #[tokio::test]
    async fn timed_out_upload_abort_releases_reservation_and_staging_file() {
        let (d, p, conversation) = fixture().await;
        let upload = MediaUpload::start(
            p.clone(),
            d.path(),
            StartMedia {
                owner_id: "u",
                intent: MediaIntent::Message {
                    conversation_id: conversation,
                },
                original_filename: "timeout.bin",
                content_type: "application/octet-stream",
                max_bytes: 8,
                per_user_bytes: 16,
                total_bytes: 64,
            },
        )
        .await
        .unwrap();
        let id = upload.id.clone();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(1),
                std::future::pending::<()>(),
            )
            .await
            .is_err()
        );
        upload.abort().await;
        let row: (String, i64) =
            sqlx::query_as("SELECT media_state,reserved_bytes FROM attachments WHERE id=?")
                .bind(&id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(row, ("failed".into(), 0));
        assert!(!d.path().join("staging").join(format!("{id}.part")).exists());
    }

    #[tokio::test]
    async fn collection_uses_upload_activity_without_rewriting_creation_time() {
        let (d, p, conversation) = fixture().await;
        let upload = MediaUpload::start(
            p.clone(),
            d.path(),
            StartMedia {
                owner_id: "u",
                intent: MediaIntent::Message {
                    conversation_id: conversation.clone(),
                },
                original_filename: "x",
                content_type: "text/plain",
                max_bytes: 8,
                per_user_bytes: 16,
                total_bytes: 64,
            },
        )
        .await
        .unwrap();
        let id = upload.id.clone();
        sqlx::query(
            "UPDATE attachments SET created_at='2000-01-01 00:00:00',upload_updated_at=datetime('now') WHERE id=?",
        )
        .bind(&id)
        .execute(&p)
        .await
        .unwrap();

        assert_eq!(collect_expired(&p, d.path(), 60).await.unwrap(), 0);
        let created_at: String =
            sqlx::query_scalar("SELECT created_at FROM attachments WHERE id=?")
                .bind(&id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(created_at, "2000-01-01 00:00:00");

        sqlx::query("UPDATE attachments SET upload_updated_at='2000-01-01 00:00:00' WHERE id=?")
            .bind(&id)
            .execute(&p)
            .await
            .unwrap();
        assert_eq!(collect_expired(&p, d.path(), 60).await.unwrap(), 1);
        let state: String = sqlx::query_scalar("SELECT media_state FROM attachments WHERE id=?")
            .bind(&id)
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(state, "failed");
    }

    #[tokio::test]
    async fn reconciles_object_renamed_before_metadata_commit() {
        let (d, p, conversation) = fixture().await;
        let mut upload = MediaUpload::start(
            p.clone(),
            d.path(),
            StartMedia {
                owner_id: "u",
                intent: MediaIntent::Message {
                    conversation_id: conversation.clone(),
                },
                original_filename: "x",
                content_type: "text/plain",
                max_bytes: 8,
                per_user_bytes: 16,
                total_bytes: 64,
            },
        )
        .await
        .unwrap();
        upload.write_chunk(b"hello").await.unwrap();
        upload.file.flush().await.unwrap();
        upload.file.sync_all().await.unwrap();
        let id = upload.id.clone();
        let final_path = upload.final_path.clone();
        rooted_rename(
            upload.root.clone(),
            upload.staging_path.clone(),
            final_path.clone(),
        )
        .await
        .unwrap();
        drop(upload.file);
        assert_eq!(reconcile_interrupted(&p, d.path()).await.unwrap(), 1);
        let row: (String, i64, String) =
            sqlx::query_as("SELECT media_state,file_size,sha256 FROM attachments WHERE id=?")
                .bind(id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(row.0, "ready");
        assert_eq!(row.1, 5);
        assert!(!row.2.is_empty());
    }

    #[test]
    fn storage_keys_cannot_escape_the_media_root() {
        assert!(safe_storage_key("objects/ab/id"));
        for key in [
            "../objects/ab/id",
            "objects/../secret",
            "objects/ab/../../secret",
            "/objects/ab/id",
            "objects\\ab\\id",
            "staging/id.part",
        ] {
            assert!(!safe_storage_key(key), "accepted unsafe key {key:?}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rooted_media_open_and_delete_reject_parent_symlink_escape() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"outside").unwrap();
        symlink(outside.path(), root.path().join("objects")).unwrap();
        assert!(
            open_rooted_media(root.path(), "objects/secret")
                .await
                .is_err()
        );
        let rooted = open_media_root(root.path()).unwrap();
        assert!(
            rooted_remove(rooted, PathBuf::from("objects/secret"))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(outside.path().join("secret")).unwrap(),
            b"outside"
        );
    }

    #[tokio::test]
    async fn historical_import_preserves_provenance_and_switches_only_verified_bytes() {
        let (d, p, conversation) = fixture().await;
        let id = Uuid::new_v4().to_string();
        let (address, server) = http_fixture(b"legacy bytes".to_vec()).await;
        let previous_url = "http://legacy.test/blob";
        sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_cid,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,? ,?,'legacy_external',?,?,1,'pending')")
            .bind(&id).bind("u").bind(&conversation).bind(&id).bind("old.bin").bind("application/octet-stream").bind(12_i64).bind("cid-old").bind(previous_url).execute(&p).await.unwrap();
        sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,previous_cid,expected_size,outcome,reference_outcome) VALUES(?,?,?,?, 'pending','not_checked')")
            .bind(&id).bind(previous_url).bind("cid-old").bind(12_i64).execute(&p).await.unwrap();
        let client = crate::egress::ControlledHttpClient::fixture(address, 64);
        let report = import_legacy_batch(&p, d.path(), &client, 64, 30, 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(
            report,
            ImportReport {
                claimed: 1,
                imported: 1,
                unresolved: 0
            }
        );
        let row: (String, String, String, i64) = sqlx::query_as(
            "SELECT media_state,storage_key,sha256,previously_public FROM attachments WHERE id=?",
        )
        .bind(&id)
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(row.0, "ready");
        assert_eq!(
            tokio::fs::read(d.path().join(&row.1)).await.unwrap(),
            b"legacy bytes"
        );
        assert_eq!(row.2, hex::encode(Sha256::digest(b"legacy bytes")));
        assert_eq!(row.3, 1);
        let inventory = external_reference_inventory(&p).await.unwrap();
        assert_eq!(inventory[0].previous_url, previous_url);
        assert_eq!(inventory[0].previous_cid.as_deref(), Some("cid-old"));
        assert_eq!(inventory[0].outcome, "imported");
    }

    #[tokio::test]
    async fn historical_import_records_size_mismatch_without_switching_locator() {
        let (d, p, conversation) = fixture().await;
        let id = Uuid::new_v4().to_string();
        let (address, server) = http_fixture(b"different".to_vec()).await;
        let previous_url = "http://legacy.test/blob";
        sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,? ,?,'legacy_external',?,1,'pending')")
            .bind(&id).bind("u").bind(&conversation).bind(&id).bind("old.bin").bind("application/octet-stream").bind(99_i64).bind(previous_url).execute(&p).await.unwrap();
        sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,99,'pending')").bind(&id).bind(previous_url).execute(&p).await.unwrap();
        let client = crate::egress::ControlledHttpClient::fixture(address, 64);
        let report = import_legacy_batch(&p, d.path(), &client, 64, 30, 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(report.unresolved, 1);
        let row: (String, Option<String>) =
            sqlx::query_as("SELECT media_state,storage_key FROM attachments WHERE id=?")
                .bind(&id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(row, ("legacy_external".into(), None));
        let outcome: String =
            sqlx::query_scalar("SELECT outcome FROM media_import_ledger WHERE attachment_id=?")
                .bind(id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(outcome, "size_mismatch");
    }

    #[tokio::test]
    async fn historical_import_streams_above_preview_body_limit() {
        let (d, p, conversation) = fixture().await;
        let id = Uuid::new_v4().to_string();
        let body = vec![0x5a; 3 * 1024 * 1024 + 7];
        let size = body.len() as i64;
        let expected_hash = hex::encode(Sha256::digest(&body));
        let (address, server) = http_fixture(body).await;
        let previous_url = "http://legacy.test/large";
        sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,?,?,'legacy_external',?,1,'pending')")
            .bind(&id).bind("u").bind(&conversation).bind(&id).bind("large.bin").bind("application/octet-stream").bind(size).bind(previous_url).execute(&p).await.unwrap();
        sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,?,'pending')").bind(&id).bind(previous_url).bind(size).execute(&p).await.unwrap();
        let client = crate::egress::ControlledHttpClient::fixture(address, 4 * 1024 * 1024);
        let report = import_legacy_batch(&p, d.path(), &client, 4 * 1024 * 1024, 30, 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(report.imported, 1);
        let (key, hash): (String, String) =
            sqlx::query_as("SELECT storage_key,sha256 FROM attachments WHERE id=?")
                .bind(id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(hash, expected_hash);
        assert_eq!(
            std::fs::metadata(d.path().join(key)).unwrap().len(),
            size as u64
        );
    }

    #[tokio::test]
    async fn fragmented_import_has_bounded_lease_writes() {
        let (d, p, conversation) = fixture().await;
        let id = Uuid::new_v4().to_string();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            for _ in 0..500 {
                stream.write_all(b"x").await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let previous_url = "http://legacy.test/fragments";
        sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,?,500,'legacy_external',?,1,'pending')")
            .bind(&id).bind("u").bind(&conversation).bind(&id).bind("fragments.bin").bind("application/octet-stream").bind(previous_url).execute(&p).await.unwrap();
        sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,500,'pending')")
            .bind(&id).bind(previous_url).execute(&p).await.unwrap();
        sqlx::query("CREATE TABLE lease_updates(count INTEGER NOT NULL)")
            .execute(&p)
            .await
            .unwrap();
        sqlx::query("INSERT INTO lease_updates(count) VALUES(0)")
            .execute(&p)
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER count_lease_updates AFTER UPDATE OF claim_until ON media_import_ledger WHEN OLD.outcome='importing' AND NEW.outcome='importing' BEGIN UPDATE lease_updates SET count=count+1; END")
            .execute(&p).await.unwrap();

        let client = crate::egress::ControlledHttpClient::fixture(address, 1_000);
        let report = import_legacy_batch(&p, d.path(), &client, 1_000, 30, 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(report.imported, 1);
        let renewals: i64 = sqlx::query_scalar("SELECT count FROM lease_updates")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(
            renewals, 1,
            "fragment count must not determine SQLite write count"
        );
    }

    #[tokio::test]
    async fn expired_import_attempt_cannot_overwrite_newer_verified_bytes() {
        let (d, p, conversation) = fixture().await;
        let id = Uuid::new_v4().to_string();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let first_headers_sent = Arc::new(tokio::sync::Notify::new());
        let release_first_body = Arc::new(tokio::sync::Notify::new());
        let server = {
            let first_headers_sent = first_headers_sent.clone();
            let release_first_body = release_first_body.clone();
            tokio::spawn(async move {
                let (mut first, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 2048];
                let _ = first.read(&mut request).await;
                first
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                first_headers_sent.notify_one();

                let (mut second, _) = listener.accept().await.unwrap();
                let _ = second.read(&mut request).await;
                second
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nnewer",
                    )
                    .await
                    .unwrap();

                release_first_body.notified().await;
                first.write_all(b"older").await.unwrap();
            })
        };
        let previous_url = "http://legacy.test/raced";
        sqlx::query("INSERT INTO attachments(id,uploader_id,conversation_id,filename,original_filename,content_type,file_size,media_state,blob_url,previously_public,import_outcome) VALUES(?,?,?,?,?,?,5,'legacy_external',?,1,'pending')")
            .bind(&id).bind("u").bind(&conversation).bind(&id).bind("raced.bin").bind("application/octet-stream").bind(previous_url).execute(&p).await.unwrap();
        sqlx::query("INSERT INTO media_import_ledger(attachment_id,previous_url,expected_size,outcome) VALUES(?,?,5,'pending')")
            .bind(&id).bind(previous_url).execute(&p).await.unwrap();
        let client = crate::egress::ControlledHttpClient::fixture(address, 64);

        let stale = {
            let p = p.clone();
            let root = d.path().to_owned();
            let client = client.clone();
            tokio::spawn(async move { import_legacy_batch(&p, &root, &client, 64, 1, 1).await })
        };
        first_headers_sent.notified().await;
        sqlx::query("UPDATE media_import_ledger SET claim_until=datetime('now','-1 second') WHERE attachment_id=? AND outcome='importing'")
            .bind(&id)
            .execute(&p)
            .await
            .unwrap();
        let winner_client = crate::egress::ControlledHttpClient::fixture(address, 64);
        let winner = import_legacy_batch(&p, d.path(), &winner_client, 64, 30, 1)
            .await
            .unwrap();
        assert_eq!(winner.imported, 1);
        release_first_body.notify_one();
        let stale = stale.await.unwrap().unwrap();
        server.await.unwrap();
        assert_eq!(stale.unresolved, 1);

        let (key, hash): (String, String) =
            sqlx::query_as("SELECT storage_key,sha256 FROM attachments WHERE id=?")
                .bind(&id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(
            tokio::fs::read(d.path().join(&key)).await.unwrap(),
            b"newer"
        );
        assert_eq!(hash, hex::encode(Sha256::digest(b"newer")));
        let object_count = std::fs::read_dir(d.path().join("objects"))
            .unwrap()
            .map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap().count())
            .sum::<usize>();
        assert_eq!(object_count, 1, "the stale attempt object must be removed");
    }
}
