use std::path::Path;

use sqlx::SqlitePool;

use crate::auth::authority::{Actor, AuthError, AuthService};

use crate::media::{MediaIntent, MediaUpload, ServerMediaPurpose, StartMedia, UserMediaPurpose};

use super::authorization::{AuthorizationError, AuthorizationService, ConversationAction};

use super::permissions::Permissions;

#[derive(Clone)]
pub struct MediaService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

pub struct UploadTarget<'a> {
    pub purpose: &'a str,
    pub conversation_id: Option<&'a str>,
    pub server_id: Option<&'a str>,
    pub channel: Option<&'a str>,
}

#[derive(Clone)]
enum UploadAuthorization {
    Conversation(String),
    ManagedServer {
        server_id: String,
        member_asset: bool,
    },
    OwnProfile,
}

#[derive(Clone)]
pub struct AuthorizedUpload {
    intent: MediaIntent,
    authorization: UploadAuthorization,
    pub max_bytes: u64,
    pub images_only: bool,
}

pub struct UploadReservation<'a> {
    pub media_root: &'a Path,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub per_user_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct AuthorizedDownload {
    pub original_filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub storage_key: String,
}

#[derive(Debug, sqlx::FromRow)]
struct AttachmentAccess {
    uploader_id: String,
    original_filename: String,
    content_type: String,
    file_size: i64,
    media_state: String,
    storage_key: String,
    conversation_id: Option<String>,
    managed_server_id: Option<String>,
    managed_user_id: Option<String>,
    media_purpose: String,
    deleted_at: Option<String>,
}

pub struct CreatedEmoji {
    pub id: String,
    pub name: String,
    pub image_url: String,
}

pub struct CreatedSticker {
    pub id: String,
    pub name: String,
    pub image_url: String,
    pub description: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct EmojiAsset {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub image_url: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct StickerAsset {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub image_url: String,
    pub description: Option<String>,
}

pub struct MemberMediaUpdate {
    pub nickname: Option<String>,
    pub username: String,
    pub avatar_url: String,
}

pub struct ProfileUpdate<'a> {
    pub bio: Option<&'a str>,
    pub pronouns: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub banner_url: Option<&'a str>,
}

impl MediaService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            pool,
            auth,
            writes,
        }
    }
}

async fn claim_server_asset(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    attachment_id: &str,
    user_id: &str,
    server_id: &str,
    purpose: &str,
    conflict: &'static str,
) -> Result<(), String> {
    let claimed = sqlx::query(
        "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
         WHERE id=? AND uploader_id=? AND media_state='ready' AND media_purpose=? \
         AND managed_server_id=? AND message_id IS NULL",
    )
    .bind(attachment_id)
    .bind(user_id)
    .bind(purpose)
    .bind(server_id)
    .execute(&mut **transaction)
    .await
    .map_err(dependency_error)?;
    if claimed.rows_affected() != 1 {
        return Err(format!("CONFLICT: {conflict}"));
    }
    Ok(())
}

async fn schedule_replaced_media(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    attachment_id: &str,
    purpose: &str,
    server_id: Option<&str>,
    user_id: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE attachments SET media_state='deleting',delete_after=datetime('now','+1 hour'),\
         state_version=state_version+1 WHERE id=? AND media_purpose=? \
         AND managed_server_id IS ? AND managed_user_id IS ? AND media_state='attached'",
    )
    .bind(attachment_id)
    .bind(purpose)
    .bind(server_id)
    .bind(user_id)
    .execute(&mut **transaction)
    .await
    .map_err(dependency_error)?;
    Ok(())
}

fn validate_profile(update: &ProfileUpdate<'_>) -> Result<(), String> {
    if update.bio.is_some_and(|value| value.len() > 2_000) {
        return Err("INVALID_INPUT: bio must be 2000 characters or less".into());
    }
    if update.pronouns.is_some_and(|value| value.len() > 100) {
        return Err("INVALID_INPUT: pronouns must be 100 characters or less".into());
    }
    if update.banner_url.is_some_and(|value| value.len() > 2_000) {
        return Err("INVALID_INPUT: banner URL must be 2000 characters or less".into());
    }
    Ok(())
}

fn authentication_error(error: AuthError) -> String {
    match error {
        AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_) => {
            dependency_error_message()
        }
        _ => "UNAUTHENTICATED: authentication required".into(),
    }
}

fn authorization_error(error: AuthorizationError) -> String {
    match error {
        AuthorizationError::Authentication(error) => authentication_error(error),
        AuthorizationError::Unavailable => "FORBIDDEN: resource unavailable".into(),
        AuthorizationError::Database(error) => dependency_error(error),
    }
}

fn dependency_error(_: sqlx::Error) -> String {
    dependency_error_message()
}

fn dependency_error_message() -> String {
    "DEPENDENCY_UNAVAILABLE: media dependency unavailable".into()
}

mod avatars;
mod catalog;
mod downloads;
mod profiles;
mod server_assets;
mod uploads;
