use super::{
    Actor, AttachmentAccess, AuthorizedDownload, MediaService, authentication_error,
    dependency_error, dependency_error_message,
};

impl MediaService {
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

    pub(super) async fn load_attachment(
        &self,
        attachment_id: &str,
    ) -> Result<AttachmentAccess, String> {
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

    pub(super) async fn attachment_is_authorized(
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
                    crate::engine::authorization::ConversationAction::Read,
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

    pub(super) async fn begin_write(
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
