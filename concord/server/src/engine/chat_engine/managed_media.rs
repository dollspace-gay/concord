use super::ChatEngine;

impl ChatEngine {
    pub async fn authorize_media_upload(
        &self,
        actor: &crate::auth::authority::Actor,
        target: crate::engine::media_service::UploadTarget<'_>,
        instance_max_bytes: u64,
    ) -> Result<crate::engine::media_service::AuthorizedUpload, String> {
        self.media_service()?
            .authorize_upload(actor, target, instance_max_bytes)
            .await
    }
    pub async fn reserve_media_upload(
        &self,
        actor: &crate::auth::authority::Actor,
        plan: crate::engine::media_service::AuthorizedUpload,
        request: crate::engine::media_service::UploadReservation<'_>,
    ) -> Result<crate::media::MediaUpload, crate::media::MediaError> {
        let service = self
            .media_service()
            .map_err(|_| crate::media::MediaError::Invalid)?;
        service.reserve_upload(actor, plan, request).await
    }
    pub async fn update_server_icon_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        icon_url: &str,
    ) -> Result<(), String> {
        self.media_service()?
            .update_server_icon(actor, server_id, icon_url)
            .await?;
        self.load_servers_from_db()
            .await
            .map_err(|_| "DEPENDENCY_UNAVAILABLE: server state refresh failed".to_string())
    }
    pub async fn update_member_avatar_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        avatar_url: &str,
    ) -> Result<(), String> {
        let update = self
            .media_service()?
            .update_member_avatar(actor, server_id, avatar_url)
            .await?;
        let display_name = update.nickname.clone().unwrap_or(update.username);
        self.broadcast_server_member_identity(
            server_id,
            actor.user_id().as_str(),
            update.nickname,
            display_name,
            Some(update.avatar_url),
        );
        Ok(())
    }
    pub async fn create_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        name: &str,
        image_url: &str,
    ) -> Result<crate::engine::media_service::CreatedEmoji, String> {
        self.media_service()?
            .create_emoji(actor, server_id, name, image_url)
            .await
    }
    pub async fn list_server_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<crate::engine::media_service::EmojiAsset>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.media_service()?
            .list_server_emoji(actor, server_id)
            .await
    }
    pub async fn list_user_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        target_server_id: &str,
    ) -> Result<
        (
            Vec<crate::engine::media_service::EmojiAsset>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.media_service()?
            .list_user_emoji(actor, target_server_id)
            .await
    }
    pub async fn delete_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        emoji_id: &str,
    ) -> Result<bool, String> {
        self.media_service()?
            .delete_emoji(actor, server_id, emoji_id)
            .await
    }
    pub async fn create_sticker_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        name: &str,
        image_url: &str,
        description: Option<&str>,
    ) -> Result<crate::engine::media_service::CreatedSticker, String> {
        self.media_service()?
            .create_sticker(actor, server_id, name, image_url, description)
            .await
    }
    pub async fn list_server_stickers_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<crate::engine::media_service::StickerAsset>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.media_service()?
            .list_server_stickers(actor, server_id)
            .await
    }
    pub async fn delete_sticker_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        sticker_id: &str,
    ) -> Result<bool, String> {
        self.media_service()?
            .delete_sticker(actor, server_id, sticker_id)
            .await
    }
    pub async fn update_profile_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        update: crate::engine::media_service::ProfileUpdate<'_>,
    ) -> Result<(), String> {
        self.media_service()?.update_profile(actor, update).await?;
        let (profile, _) = self
            .get_user_profile(actor, actor.user_id().as_str())
            .await
            .map_err(|_| "DEPENDENCY_UNAVAILABLE: profile refresh failed".to_string())?;
        self.broadcast_profile_update(profile);
        Ok(())
    }
    pub async fn authorized_media_download(
        &self,
        actor: &crate::auth::authority::Actor,
        attachment_id: &str,
    ) -> Result<crate::engine::media_service::AuthorizedDownload, String> {
        self.media_service()?
            .authorized_download(actor, attachment_id)
            .await
    }
    pub async fn media_download_is_still_authorized(
        &self,
        actor: &crate::auth::authority::Actor,
        attachment_id: &str,
    ) -> bool {
        let Ok(service) = self.media_service() else {
            return false;
        };
        service
            .download_is_still_authorized(actor, attachment_id)
            .await
    }
    pub async fn delete_unattached_upload_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        attachment_id: &str,
    ) -> Result<bool, String> {
        self.media_service()?
            .delete_unattached_upload(actor, attachment_id)
            .await
    }
}
