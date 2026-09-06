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

    pub async fn list_server_emoji(
        &self,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(Vec<EmojiAsset>, super::authorization::AuthorizationStamp), String> {
        let mut transaction = self.pool.begin().await.map_err(dependency_error)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await
            .map_err(authorization_error)?;
        let rows = sqlx::query_as::<_, EmojiAsset>(
            "SELECT id,server_id,name,image_url FROM custom_emoji WHERE server_id=? ORDER BY name",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut transaction, server_id, &[])
            .await
            .map_err(authorization_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok((rows, stamp))
    }

    pub async fn list_server_stickers(
        &self,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(Vec<StickerAsset>, super::authorization::AuthorizationStamp), String> {
        let mut transaction = self.pool.begin().await.map_err(dependency_error)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await
            .map_err(authorization_error)?;
        let rows = sqlx::query_as::<_, StickerAsset>(
            "SELECT id,server_id,name,image_url,description FROM stickers WHERE server_id=? ORDER BY name",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut transaction, server_id, &[])
            .await
            .map_err(authorization_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok((rows, stamp))
    }

    pub async fn list_user_emoji(
        &self,
        actor: &Actor,
        target_server_id: &str,
    ) -> Result<(Vec<EmojiAsset>, super::authorization::AuthorizationStamp), String> {
        let mut transaction = self.pool.begin().await.map_err(dependency_error)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                target_server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await
            .map_err(authorization_error)?;
        let rows = sqlx::query_as::<_, EmojiAsset>(
            "SELECT e.id,e.server_id,e.name,e.image_url FROM custom_emoji e \
             JOIN servers s ON e.server_id=s.id \
             JOIN server_members sm ON s.id=sm.server_id \
             JOIN servers target ON target.id=? \
             WHERE sm.user_id=? AND s.shareable_emoji=1 \
             AND (e.server_id=target.id OR target.allow_external_emoji=1) \
             ORDER BY s.name,e.name",
        )
        .bind(target_server_id)
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut transaction, target_server_id, &[])
            .await
            .map_err(authorization_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok((rows, stamp))
    }

    pub async fn authorize_upload(
        &self,
        actor: &Actor,
        target: UploadTarget<'_>,
        instance_max_bytes: u64,
    ) -> Result<AuthorizedUpload, String> {
        let purpose = target.purpose;
        let (intent, authorization, purpose_limit, images_only) = match purpose {
            "message" => {
                let conversation_id = match target.conversation_id {
                    Some(conversation_id)
                        if target.server_id.is_none() && target.channel.is_none() =>
                    {
                        conversation_id.to_owned()
                    }
                    Some(_) => {
                        return Err(
                            "INVALID_INPUT: use conversation_id or the legacy server/channel target"
                                .into(),
                        );
                    }
                    None => {
                        let (Some(server_id), Some(channel)) = (target.server_id, target.channel)
                        else {
                            return Err(
                                "INVALID_INPUT: message uploads require conversation_id or server_id and channel"
                                    .into(),
                            );
                        };
                        sqlx::query_scalar(
                            "SELECT v.id FROM channels c \
                             JOIN conversations v ON v.channel_id=c.id \
                             WHERE c.server_id=? AND (c.id=? OR c.name=?) LIMIT 1",
                        )
                        .bind(server_id)
                        .bind(channel)
                        .bind(channel)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(dependency_error)?
                        .ok_or_else(|| "FORBIDDEN: resource unavailable".to_string())?
                    }
                };
                (
                    MediaIntent::Message {
                        conversation_id: conversation_id.clone(),
                    },
                    UploadAuthorization::Conversation(conversation_id),
                    instance_max_bytes,
                    false,
                )
            }
            "emoji" | "sticker" | "server_avatar" | "server_member_avatar" => {
                let server_id = target
                    .server_id
                    .ok_or_else(|| "INVALID_INPUT: server media requires server_id".to_string())?
                    .to_owned();
                let (asset_purpose, limit) = match purpose {
                    "emoji" => (ServerMediaPurpose::Emoji, 256 * 1024),
                    "sticker" => (ServerMediaPurpose::Sticker, 512 * 1024),
                    "server_avatar" => (ServerMediaPurpose::Avatar, 8 * 1024 * 1024),
                    _ => (ServerMediaPurpose::MemberAvatar, 8 * 1024 * 1024),
                };
                (
                    MediaIntent::ServerAsset {
                        server_id: server_id.clone(),
                        purpose: asset_purpose,
                    },
                    UploadAuthorization::ManagedServer {
                        server_id,
                        member_asset: purpose == "server_member_avatar",
                    },
                    instance_max_bytes.min(limit),
                    true,
                )
            }
            "user_avatar" | "user_banner" => {
                let (asset_purpose, limit) = if purpose == "user_avatar" {
                    (UserMediaPurpose::Avatar, 8 * 1024 * 1024)
                } else {
                    (UserMediaPurpose::Banner, 16 * 1024 * 1024)
                };
                (
                    MediaIntent::UserAsset {
                        purpose: asset_purpose,
                    },
                    UploadAuthorization::OwnProfile,
                    instance_max_bytes.min(limit),
                    true,
                )
            }
            _ => return Err("INVALID_INPUT: unknown media purpose".into()),
        };
        self.authorize_upload_scope(actor, &authorization).await?;
        Ok(AuthorizedUpload {
            intent,
            authorization,
            max_bytes: purpose_limit,
            images_only,
        })
    }

    pub async fn reserve_upload(
        &self,
        actor: &Actor,
        plan: AuthorizedUpload,
        request: UploadReservation<'_>,
    ) -> Result<MediaUpload, crate::media::MediaError> {
        self.authorize_upload_scope(actor, &plan.authorization)
            .await
            .map_err(|_| crate::media::MediaError::Invalid)?;
        MediaUpload::start(
            self.pool.clone(),
            request.media_root,
            StartMedia {
                owner_id: actor.user_id().as_str(),
                intent: plan.intent,
                original_filename: request.filename,
                content_type: request.content_type,
                max_bytes: plan.max_bytes,
                per_user_bytes: request.per_user_bytes,
                total_bytes: request.total_bytes,
            },
        )
        .await
    }

    async fn authorize_upload_scope(
        &self,
        actor: &Actor,
        authorization: &UploadAuthorization,
    ) -> Result<(), String> {
        let mut connection = self.pool.acquire().await.map_err(dependency_error)?;
        match authorization {
            UploadAuthorization::Conversation(conversation_id) => self
                .authorization
                .authorize_conversation_actor_in(
                    &mut connection,
                    &self.auth,
                    actor,
                    conversation_id,
                    ConversationAction::Send,
                )
                .await
                .map(|_| ())
                .map_err(authorization_error),
            UploadAuthorization::ManagedServer {
                server_id,
                member_asset,
            } => {
                if *member_asset {
                    self.authorization
                        .server_actor_permissions_in(&mut connection, &self.auth, actor, server_id)
                        .await
                        .map(|_| ())
                        .map_err(authorization_error)
                } else {
                    self.authorization
                        .require_server_actor_in(
                            &mut connection,
                            &self.auth,
                            actor,
                            server_id,
                            Permissions::MANAGE_SERVER,
                        )
                        .await
                        .map(|_| ())
                        .map_err(authorization_error)
                }
            }
            UploadAuthorization::OwnProfile => self
                .auth
                .validate_actor_in(&mut connection, actor)
                .await
                .map_err(authentication_error),
        }
    }

    pub async fn update_server_icon(
        &self,
        actor: &Actor,
        server_id: &str,
        icon_url: &str,
    ) -> Result<(), String> {
        let attachment_id = crate::media::local_attachment_id(icon_url).ok_or_else(|| {
            "INVALID_INPUT: server icon must be a managed local upload".to_string()
        })?;
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(authorization_error)?;
        let previous_icon: Option<String> =
            sqlx::query_scalar("SELECT icon_url FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(dependency_error)?;
        let claimed = sqlx::query(
            "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
             WHERE id=? AND uploader_id=? AND managed_server_id=? \
             AND media_purpose='server_avatar' AND media_state='ready' AND message_id IS NULL",
        )
        .bind(attachment_id)
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        if claimed.rows_affected() != 1 {
            return Err("CONFLICT: server icon upload is unavailable or already claimed".into());
        }
        sqlx::query("UPDATE servers SET icon_url=?,updated_at=datetime('now') WHERE id=?")
            .bind(icon_url)
            .bind(server_id)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        if previous_icon.as_deref() != Some(icon_url)
            && let Some(previous_id) = previous_icon
                .as_deref()
                .and_then(crate::media::local_attachment_id)
        {
            schedule_replaced_media(
                &mut transaction,
                previous_id,
                "server_avatar",
                Some(server_id),
                None,
            )
            .await?;
        }
        transaction.commit().await.map_err(dependency_error)?;
        Ok(())
    }

    pub async fn update_member_avatar(
        &self,
        actor: &Actor,
        server_id: &str,
        avatar_url: &str,
    ) -> Result<MemberMediaUpdate, String> {
        let attachment_id = crate::media::local_attachment_id(avatar_url).ok_or_else(|| {
            "INVALID_INPUT: member avatar must be a managed local upload".to_string()
        })?;
        let user_id = actor.user_id().as_str();
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .server_actor_permissions_in(&mut transaction, &self.auth, actor, server_id)
            .await
            .map_err(authorization_error)?;
        let previous_avatar: Option<String> = sqlx::query_scalar(
            "SELECT avatar_url FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        let claimed = sqlx::query(
            "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
             WHERE id=? AND uploader_id=? AND managed_user_id=? AND managed_server_id=? \
             AND media_purpose='server_member_avatar' AND media_state='ready' AND message_id IS NULL",
        )
        .bind(attachment_id)
        .bind(user_id)
        .bind(user_id)
        .bind(server_id)
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        if claimed.rows_affected() != 1 {
            return Err("CONFLICT: member avatar upload is unavailable or already claimed".into());
        }
        sqlx::query("UPDATE server_members SET avatar_url=? WHERE server_id=? AND user_id=?")
            .bind(avatar_url)
            .bind(server_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        if previous_avatar.as_deref() != Some(avatar_url)
            && let Some(previous_id) = previous_avatar
                .as_deref()
                .and_then(crate::media::local_attachment_id)
        {
            schedule_replaced_media(
                &mut transaction,
                previous_id,
                "server_member_avatar",
                Some(server_id),
                Some(user_id),
            )
            .await?;
        }
        let (nickname, username): (Option<String>, String) = sqlx::query_as(
            "SELECT sm.nickname,u.username FROM server_members sm \
             JOIN users u ON u.id=sm.user_id WHERE sm.server_id=? AND sm.user_id=?",
        )
        .bind(server_id)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(MemberMediaUpdate {
            nickname,
            username,
            avatar_url: avatar_url.to_owned(),
        })
    }

    pub async fn create_emoji(
        &self,
        actor: &Actor,
        server_id: &str,
        name: &str,
        image_url: &str,
    ) -> Result<CreatedEmoji, String> {
        let name = name.trim().to_lowercase();
        if name.len() < 2
            || name.len() > 32
            || !name
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
        {
            return Err(
                "INVALID_INPUT: emoji name must be 2-32 alphanumeric/underscore characters".into(),
            );
        }
        let attachment_id = crate::media::local_attachment_id(image_url).ok_or_else(|| {
            "INVALID_INPUT: emoji image must be a managed local upload".to_string()
        })?;
        let id = uuid::Uuid::new_v4().to_string();
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(authorization_error)?;
        claim_server_asset(
            &mut transaction,
            attachment_id,
            actor.user_id().as_str(),
            server_id,
            "emoji",
            "emoji upload is unavailable or already claimed",
        )
        .await?;
        sqlx::query(
            "INSERT INTO custom_emoji(id,server_id,name,image_url,uploader_id) VALUES(?,?,?,?,?)",
        )
        .bind(&id)
        .bind(server_id)
        .bind(&name)
        .bind(image_url)
        .bind(actor.user_id().as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            if error.to_string().contains("UNIQUE") {
                "CONFLICT: an emoji with that name already exists".into()
            } else {
                dependency_error(error)
            }
        })?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(CreatedEmoji {
            id,
            name,
            image_url: image_url.to_owned(),
        })
    }

    pub async fn delete_emoji(
        &self,
        actor: &Actor,
        server_id: &str,
        emoji_id: &str,
    ) -> Result<bool, String> {
        self.delete_server_asset(actor, server_id, emoji_id, "emoji")
            .await
    }

    pub async fn create_sticker(
        &self,
        actor: &Actor,
        server_id: &str,
        name: &str,
        image_url: &str,
        description: Option<&str>,
    ) -> Result<CreatedSticker, String> {
        let name = name.trim().to_lowercase();
        if name.is_empty()
            || name.len() > 32
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(
                "INVALID_INPUT: sticker name must be 1-32 alphanumeric/underscore characters"
                    .into(),
            );
        }
        if description.is_some_and(|value| value.chars().count() > 100) {
            return Err("INVALID_INPUT: sticker description must be at most 100 characters".into());
        }
        let attachment_id = crate::media::local_attachment_id(image_url).ok_or_else(|| {
            "INVALID_INPUT: sticker image must be a managed local upload".to_string()
        })?;
        let id = uuid::Uuid::new_v4().to_string();
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(authorization_error)?;
        claim_server_asset(
            &mut transaction,
            attachment_id,
            actor.user_id().as_str(),
            server_id,
            "sticker",
            "sticker upload is unavailable or already claimed",
        )
        .await?;
        sqlx::query(
            "INSERT INTO stickers(id,server_id,name,image_url,description,uploader_id) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(server_id)
        .bind(&name)
        .bind(image_url)
        .bind(description)
        .bind(actor.user_id().as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            if error.to_string().contains("UNIQUE") {
                "CONFLICT: a sticker with that name already exists".into()
            } else {
                dependency_error(error)
            }
        })?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(CreatedSticker {
            id,
            name,
            image_url: image_url.to_owned(),
            description: description.map(str::to_owned),
        })
    }

    pub async fn delete_sticker(
        &self,
        actor: &Actor,
        server_id: &str,
        sticker_id: &str,
    ) -> Result<bool, String> {
        self.delete_server_asset(actor, server_id, sticker_id, "sticker")
            .await
    }

    async fn delete_server_asset(
        &self,
        actor: &Actor,
        server_id: &str,
        asset_id: &str,
        purpose: &str,
    ) -> Result<bool, String> {
        let (table, missing) = match purpose {
            "emoji" => ("custom_emoji", "emoji"),
            "sticker" => ("stickers", "sticker"),
            _ => return Err("INVALID_INPUT: invalid managed media purpose".into()),
        };
        let (_permit, mut transaction) = self.begin_write().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(authorization_error)?;
        let select = format!("SELECT image_url FROM {table} WHERE id=? AND server_id=?");
        let url: Option<String> = sqlx::query_scalar(&select)
            .bind(asset_id)
            .bind(server_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        let Some(url) = url else {
            return Ok(false);
        };
        let delete = format!("DELETE FROM {table} WHERE id=? AND server_id=?");
        sqlx::query(&delete)
            .bind(asset_id)
            .bind(server_id)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        if let Some(attachment_id) = crate::media::local_attachment_id(&url) {
            schedule_replaced_media(
                &mut transaction,
                attachment_id,
                missing,
                Some(server_id),
                None,
            )
            .await?;
        }
        transaction.commit().await.map_err(dependency_error)?;
        Ok(true)
    }

    pub async fn update_profile(
        &self,
        actor: &Actor,
        update: ProfileUpdate<'_>,
    ) -> Result<(), String> {
        validate_profile(&update)?;
        let avatar_id = update
            .avatar_url
            .map(|url| {
                crate::media::local_attachment_id(url).ok_or_else(|| {
                    "INVALID_INPUT: avatar must be a managed local upload".to_string()
                })
            })
            .transpose()?;
        let banner_id = update
            .banner_url
            .map(|url| {
                crate::media::local_attachment_id(url).ok_or_else(|| {
                    "INVALID_INPUT: banner must be a managed local upload".to_string()
                })
            })
            .transpose()?;
        let user_id = actor.user_id().as_str();
        let (_permit, mut transaction) = self.begin_write().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(authentication_error)?;
        let previous_avatar: Option<String> =
            sqlx::query_scalar("SELECT avatar_url FROM users WHERE id=?")
                .bind(user_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(dependency_error)?;
        let previous_banner: Option<String> =
            sqlx::query_scalar("SELECT banner_url FROM user_profiles WHERE user_id=?")
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(dependency_error)?
                .flatten();
        for (attachment_id, purpose) in [(avatar_id, "user_avatar"), (banner_id, "user_banner")] {
            if let Some(attachment_id) = attachment_id {
                let claimed = sqlx::query(
                    "UPDATE attachments SET media_state='attached',state_version=state_version+1 \
                     WHERE id=? AND uploader_id=? AND managed_user_id=? AND media_purpose=? \
                     AND media_state='ready' AND message_id IS NULL",
                )
                .bind(attachment_id)
                .bind(user_id)
                .bind(user_id)
                .bind(purpose)
                .execute(&mut *transaction)
                .await
                .map_err(dependency_error)?;
                if claimed.rows_affected() != 1 {
                    return Err("CONFLICT: profile media is unavailable or already claimed".into());
                }
            }
        }
        sqlx::query(
            "INSERT INTO user_profiles(user_id,bio,pronouns,banner_url) VALUES(?,?,?,?) \
             ON CONFLICT(user_id) DO UPDATE SET bio=excluded.bio,pronouns=excluded.pronouns,\
             banner_url=excluded.banner_url,updated_at=datetime('now')",
        )
        .bind(user_id)
        .bind(update.bio)
        .bind(update.pronouns)
        .bind(update.banner_url)
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        if update.avatar_url.is_some() {
            sqlx::query("UPDATE users SET avatar_url=? WHERE id=?")
                .bind(update.avatar_url)
                .bind(user_id)
                .execute(&mut *transaction)
                .await
                .map_err(dependency_error)?;
        }
        for (previous, replacement, purpose) in [
            (previous_avatar.as_deref(), update.avatar_url, "user_avatar"),
            (previous_banner.as_deref(), update.banner_url, "user_banner"),
        ] {
            if replacement.is_some()
                && previous != replacement
                && let Some(previous_id) = previous.and_then(crate::media::local_attachment_id)
            {
                schedule_replaced_media(
                    &mut transaction,
                    previous_id,
                    purpose,
                    None,
                    Some(user_id),
                )
                .await?;
            }
        }
        transaction.commit().await.map_err(dependency_error)?;
        Ok(())
    }

    pub async fn authorized_download(
        &self,
        actor: &Actor,
        attachment_id: &str,
    ) -> Result<AuthorizedDownload, String> {
        let attachment = self.load_attachment(attachment_id).await?;
        if !crate::media::safe_storage_key(&attachment.storage_key)
            || !matches!(attachment.media_state.as_str(), "ready" | "attached")
            || attachment.file_size <= 0
            || attachment.deleted_at.is_some()
            || !self
                .attachment_is_authorized(actor, attachment_id, &attachment)
                .await?
        {
            return Err("FORBIDDEN: resource unavailable".into());
        }
        Ok(AuthorizedDownload {
            original_filename: attachment.original_filename,
            content_type: attachment.content_type,
            file_size: attachment.file_size,
            storage_key: attachment.storage_key,
        })
    }

    pub async fn download_is_still_authorized(&self, actor: &Actor, attachment_id: &str) -> bool {
        self.authorized_download(actor, attachment_id).await.is_ok()
    }

    pub async fn delete_unattached_upload(
        &self,
        actor: &Actor,
        attachment_id: &str,
    ) -> Result<bool, String> {
        let (_permit, mut transaction) = self.begin_write().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(authentication_error)?;
        let result = sqlx::query(
            "UPDATE attachments SET media_state='deleting',state_version=state_version+1,\
             delete_after=datetime('now','+1 hour') \
             WHERE id=? AND uploader_id=? AND media_state='ready' AND message_id IS NULL",
        )
        .bind(attachment_id)
        .bind(actor.user_id().as_str())
        .execute(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn load_attachment(&self, attachment_id: &str) -> Result<AttachmentAccess, String> {
        sqlx::query_as(
            "SELECT a.uploader_id,a.original_filename,a.content_type,a.file_size,a.media_state,\
             a.storage_key,a.conversation_id,a.managed_server_id,a.managed_user_id,a.media_purpose,\
             m.deleted_at FROM attachments a LEFT JOIN messages m ON m.id=a.message_id WHERE a.id=?",
        )
        .bind(attachment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dependency_error)?
        .ok_or_else(|| "FORBIDDEN: resource unavailable".to_string())
    }

    async fn attachment_is_authorized(
        &self,
        actor: &Actor,
        attachment_id: &str,
        media: &AttachmentAccess,
    ) -> Result<bool, String> {
        if media.media_state == "ready" {
            return Ok(media.uploader_id == actor.user_id().as_str()
                && self.auth.validate_actor(actor).await.is_ok());
        }
        let mut connection = self.pool.acquire().await.map_err(dependency_error)?;
        if media.media_purpose == "message" {
            let Some(conversation_id) = media.conversation_id.as_deref() else {
                return Ok(false);
            };
            return Ok(self
                .authorization
                .authorize_conversation_actor_in(
                    &mut connection,
                    &self.auth,
                    actor,
                    conversation_id,
                    super::authorization::ConversationAction::Read,
                )
                .await
                .is_ok());
        }
        if matches!(
            media.media_purpose.as_str(),
            "emoji" | "sticker" | "server_avatar" | "server_member_avatar"
        ) {
            let Some(server_id) = media.managed_server_id.as_deref() else {
                return Ok(false);
            };
            if self
                .authorization
                .server_actor_permissions_in(&mut connection, &self.auth, actor, server_id)
                .await
                .is_err()
            {
                return Ok(false);
            }
            let local_url = format!("/api/uploads/{attachment_id}");
            let exists = match media.media_purpose.as_str() {
                "emoji" => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM custom_emoji WHERE server_id=? AND image_url=?)",
                )
                .bind(server_id)
                .bind(&local_url)
                .fetch_one(&mut *connection)
                .await
                .map_err(dependency_error)?,
                "sticker" => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM stickers WHERE server_id=? AND image_url=?)",
                )
                .bind(server_id)
                .bind(&local_url)
                .fetch_one(&mut *connection)
                .await
                .map_err(dependency_error)?,
                "server_avatar" => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM servers WHERE id=? AND icon_url=?)",
                )
                .bind(server_id)
                .bind(&local_url)
                .fetch_one(&mut *connection)
                .await
                .map_err(dependency_error)?,
                _ => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM server_members \
                         WHERE server_id=? AND user_id=? AND avatar_url=?)",
                )
                .bind(server_id)
                .bind(media.managed_user_id.as_deref().unwrap_or_default())
                .bind(&local_url)
                .fetch_one(&mut *connection)
                .await
                .map_err(dependency_error)?,
            };
            return Ok(exists);
        }
        if matches!(media.media_purpose.as_str(), "user_avatar" | "user_banner") {
            let Some(profile_user_id) = media.managed_user_id.as_deref() else {
                return Ok(false);
            };
            self.auth
                .validate_actor_in(&mut connection, actor)
                .await
                .map_err(authentication_error)?;
            if actor.user_id().as_str() != profile_user_id {
                let shared: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM servers s \
                     JOIN server_members requester ON requester.server_id=s.id AND requester.user_id=? \
                     JOIN server_members target ON target.server_id=s.id AND target.user_id=? \
                     WHERE NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id IN (?,?)))",
                )
                .bind(actor.user_id().as_str())
                .bind(profile_user_id)
                .bind(actor.user_id().as_str())
                .bind(profile_user_id)
                .fetch_one(&mut *connection)
                .await
                .map_err(dependency_error)?;
                if !shared {
                    return Ok(false);
                }
            }
            let local_url = format!("/api/uploads/{attachment_id}");
            let exists: bool = if media.media_purpose == "user_avatar" {
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND avatar_url=?)")
                    .bind(profile_user_id)
                    .bind(local_url)
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(dependency_error)?
            } else {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM user_profiles WHERE user_id=? AND banner_url=?)",
                )
                .bind(profile_user_id)
                .bind(local_url)
                .fetch_one(&mut *connection)
                .await
                .map_err(dependency_error)?
            };
            return Ok(exists);
        }
        Ok(false)
    }

    async fn begin_write(
        &self,
    ) -> Result<
        (
            tokio::sync::OwnedSemaphorePermit,
            sqlx::Transaction<'static, sqlx::Sqlite>,
        ),
        String,
    > {
        self.writes
            .begin()
            .await
            .map_err(|_| dependency_error_message())
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
