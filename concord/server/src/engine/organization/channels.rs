use super::{
    Actor, ChannelId, CreateChannel, OrganizationError, OrganizationService, Permissions, ServerId,
};

impl OrganizationService {
    pub async fn create_channel(
        &self,
        actor: &Actor,
        command: CreateChannel<'_>,
    ) -> Result<(), OrganizationError> {
        let CreateChannel {
            server_id,
            channel_id,
            name,
            category_id,
            is_private,
            channel_type,
        } = command;
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !matches!(channel_type, "text" | "forum") {
            return Err(OrganizationError::InvalidInput("invalid channel type"));
        }
        crate::engine::validation::validate_channel_name(name)
            .map_err(|_| OrganizationError::InvalidInput("invalid channel name"))?;
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        if let Some(category_id) = category_id {
            let valid: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM channel_categories WHERE id=? AND server_id=?)",
            )
            .bind(category_id)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await?;
            if !valid {
                return Err(OrganizationError::Forbidden);
            }
        }
        let position: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position),-1)+1 FROM channels WHERE server_id=? \
             AND parent_channel_id IS NULL AND channel_type IN ('text','forum')",
        )
        .bind(server_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,category_id,position,is_private,channel_type) \
             VALUES(?,?,?,?,?,?,?)",
        )
        .bind(channel_id)
        .bind(server_id)
        .bind(name)
        .bind(category_id)
        .bind(position)
        .bind(i64::from(is_private))
        .bind(channel_type)
        .execute(&mut *tx)
        .await?;
        let alias = name.trim_start_matches('#').to_ascii_lowercase();
        sqlx::query("INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES(?,?,?)")
            .bind(server_id)
            .bind(alias)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        if is_private {
            sqlx::query(
                "INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) \
                 VALUES(?,'user',?)",
            )
            .bind(channel_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_channel(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let is_default: i64 = sqlx::query_scalar(
            "SELECT is_default FROM channels WHERE id=? AND server_id=? \
             AND parent_channel_id IS NULL AND channel_type IN ('text','forum')",
        )
        .bind(channel_id)
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OrganizationError::Forbidden)?;
        if is_default != 0 {
            let replacement: Option<String> = sqlx::query_scalar(
                "SELECT id FROM channels WHERE server_id=? AND id!=? \
                 AND parent_channel_id IS NULL AND channel_type IN ('text','forum') \
                 ORDER BY position,id LIMIT 1",
            )
            .bind(server_id)
            .bind(channel_id)
            .fetch_optional(&mut *tx)
            .await?;
            let replacement = replacement.ok_or(OrganizationError::InvalidInput(
                "cannot delete the only default channel",
            ))?;
            sqlx::query("UPDATE channels SET is_default=1 WHERE id=? AND server_id=?")
                .bind(replacement)
                .bind(server_id)
                .execute(&mut *tx)
                .await?;
        }
        let deleted = sqlx::query("DELETE FROM channels WHERE id=? AND server_id=?")
            .bind(channel_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }
}
