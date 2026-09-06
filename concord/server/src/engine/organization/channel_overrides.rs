use super::{
    Actor, ChannelId, ChannelOverrideUpdate, ChannelPermissionOverrideInfo, OrganizationError,
    OrganizationService, Permissions, ServerId,
};

impl OrganizationService {
    pub async fn list_channel_overrides(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
    ) -> Result<Vec<ChannelPermissionOverrideInfo>, OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let channel_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)")
                .bind(channel_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?;
        if !channel_exists {
            return Err(OrganizationError::Forbidden);
        }
        let rows: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT id,channel_id,target_type,target_id,allow_bits,deny_bits \
             FROM channel_permission_overrides WHERE channel_id=? \
             ORDER BY target_type,target_id",
        )
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, channel_id, target_type, target_id, allow_bits, deny_bits)| {
                    ChannelPermissionOverrideInfo {
                        id,
                        channel_id,
                        target_type,
                        target_id,
                        allow_bits,
                        deny_bits,
                    }
                },
            )
            .collect())
    }

    pub async fn set_channel_override(
        &self,
        actor: &Actor,
        update: ChannelOverrideUpdate<'_>,
    ) -> Result<(), OrganizationError> {
        let ChannelOverrideUpdate {
            server_id,
            channel_id,
            target_type,
            target_id,
            allow_bits,
            deny_bits,
        } = update;
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !matches!(target_type, "role" | "user") || target_id.trim().is_empty() {
            return Err(OrganizationError::InvalidInput("invalid override target"));
        }
        let allow = Permissions::from_bits(allow_bits as u64).ok_or(
            OrganizationError::InvalidInput("invalid override permissions"),
        )?;
        let deny = Permissions::from_bits(deny_bits as u64).ok_or(
            OrganizationError::InvalidInput("invalid override permissions"),
        )?;
        if allow.intersects(deny)
            || !(allow | deny)
                .difference(Self::CHANNEL_OVERRIDE_PERMISSIONS)
                .is_empty()
            || (allow | deny).is_empty()
        {
            return Err(OrganizationError::InvalidInput(
                "invalid override permissions",
            ));
        }

        let (_permit, mut tx) = self.writes.begin().await?;
        let actor_permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await?;
        if !actor_permissions.contains(Permissions::MANAGE_CHANNELS)
            || !actor_permissions.contains(allow)
        {
            return Err(OrganizationError::Forbidden);
        }
        let channel_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)")
                .bind(channel_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?;
        let target_exists: bool = match target_type {
            "role" => {
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roles WHERE id=? AND server_id=?)")
                    .bind(target_id)
                    .bind(server_id)
                    .fetch_one(&mut *tx)
                    .await?
            }
            "user" => {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE user_id=? AND server_id=?)",
                )
                .bind(target_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?
            }
            _ => false,
        };
        if !channel_exists || !target_exists {
            return Err(OrganizationError::Forbidden);
        }
        sqlx::query(
            "INSERT INTO channel_permission_overrides( \
                 id,channel_id,target_type,target_id,allow_bits,deny_bits \
             ) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(channel_id,target_type,target_id) DO UPDATE SET \
                 allow_bits=excluded.allow_bits,deny_bits=excluded.deny_bits",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(channel_id)
        .bind(target_type)
        .bind(target_id)
        .bind(allow_bits)
        .bind(deny_bits)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_channel_override(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        target_type: &str,
        target_id: &str,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !matches!(target_type, "role" | "user") || target_id.trim().is_empty() {
            return Err(OrganizationError::InvalidInput("invalid override target"));
        }
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
        let result = sqlx::query(
            "DELETE FROM channel_permission_overrides WHERE channel_id=? \
             AND target_type=? AND target_id=? \
             AND EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)",
        )
        .bind(channel_id)
        .bind(target_type)
        .bind(target_id)
        .bind(channel_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }
}
