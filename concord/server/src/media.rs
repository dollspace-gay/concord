use sha2::{Digest, Sha256};

use sqlx::SqlitePool;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::io::AsyncWriteExt;

use uuid::Uuid;

const MAINTENANCE_BATCH_SIZE: i64 = 100;

const UPLOAD_PROGRESS_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

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

#[cfg(test)]
mod tests;

mod legacy_import;
mod maintenance;
mod rooted_io;
pub use legacy_import::ExternalReferenceInventory;
pub use legacy_import::ImportReport;
pub use legacy_import::external_reference_inventory;
pub use legacy_import::import_legacy_batch;
pub use legacy_import::retry_legacy_import;
pub use maintenance::collect_deleted;
pub use maintenance::collect_expired;
pub(crate) use maintenance::local_attachment_id;
pub use maintenance::reconcile_interrupted;
pub(crate) use maintenance::safe_storage_key;
use rooted_io::open_media_root;
pub use rooted_io::open_rooted_media;
use rooted_io::rooted_create_dir_all;
use rooted_io::rooted_open_new;
use rooted_io::rooted_remove;
use rooted_io::rooted_rename;
use rooted_io::rooted_sync_dir;
