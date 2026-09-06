use super::{
    Actor, EmojiAsset, MediaService, Permissions, StickerAsset, authorization_error,
    dependency_error,
};

impl MediaService {
    pub async fn list_server_emoji(
        &self,
        actor: &Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<EmojiAsset>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
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
    ) -> Result<
        (
            Vec<StickerAsset>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
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
    ) -> Result<
        (
            Vec<EmojiAsset>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
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
}
