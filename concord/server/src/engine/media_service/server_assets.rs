use super::{
    Actor, CreatedEmoji, CreatedSticker, MediaService, Permissions, authorization_error,
    claim_server_asset, dependency_error, schedule_replaced_media,
};

impl MediaService {
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

    pub(super) async fn delete_server_asset(
        &self,
        actor: &Actor,
        server_id: &str,
        asset_id: &str,
        purpose: &str,
    ) -> Result<bool, String> {
        let (select, delete, missing) = match purpose {
            "emoji" => (
                "SELECT image_url FROM custom_emoji WHERE id=? AND server_id=?",
                "DELETE FROM custom_emoji WHERE id=? AND server_id=?",
                "emoji",
            ),
            "sticker" => (
                "SELECT image_url FROM stickers WHERE id=? AND server_id=?",
                "DELETE FROM stickers WHERE id=? AND server_id=?",
                "sticker",
            ),
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
        let url: Option<String> = sqlx::query_scalar(select)
            .bind(asset_id)
            .bind(server_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        let Some(url) = url else {
            return Ok(false);
        };
        sqlx::query(delete)
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
}
