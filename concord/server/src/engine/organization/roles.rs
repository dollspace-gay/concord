use super::{
    Actor, ChannelAction, ChannelId, OrganizationError, OrganizationService, Permissions, RoleRow,
    ServerId, bump_role_projection_version, highest_role_position, require_managed_role,
    require_role_grant, validate_role_fields,
};

impl OrganizationService {
    pub async fn set_topic(
        &self,
        actor: &Actor,
        channel_id: &ChannelId,
        topic: &str,
        set_by: &str,
    ) -> Result<(), OrganizationError> {
        let channel_id = channel_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .authorize_actor_in(
                &mut tx,
                &self.auth,
                actor,
                channel_id,
                ChannelAction::Manage,
            )
            .await
            .map_err(OrganizationError::from)?;
        sqlx::query(
            "UPDATE channels SET topic=?,topic_set_by=?,topic_set_at=datetime('now'),updated_at=datetime('now') WHERE id=?",
        )
        .bind(topic)
        .bind(set_by)
        .bind(channel_id)
        .execute(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(())
    }

    pub async fn list_roles(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<Vec<RoleRow>, OrganizationError> {
        let server_id = server_id.as_str();
        let mut connection = self.pool.acquire().await.map_err(OrganizationError::from)?;
        self.authorization
            .server_actor_permissions_in(&mut connection, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        sqlx::query_as("SELECT * FROM roles WHERE server_id=? ORDER BY position DESC")
            .bind(server_id)
            .fetch_all(&mut *connection)
            .await
            .map_err(OrganizationError::from)
    }

    pub async fn create_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permission_bits: i64,
    ) -> Result<RoleRow, OrganizationError> {
        let server_id = server_id.as_str();
        validate_role_fields(name, color)?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        let actor_permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        require_role_grant(actor_permissions, permission_bits)?;
        let owner: bool = sqlx::query_scalar("SELECT owner_id=? FROM servers WHERE id=?")
            .bind(actor.user_id().as_str())
            .bind(server_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(OrganizationError::from)?
            .ok_or(OrganizationError::Forbidden)?;
        let actor_highest =
            highest_role_position(&mut tx, server_id, actor.user_id().as_str(), owner).await?;
        let mut position: i32 =
            sqlx::query_scalar("SELECT COALESCE(MAX(position),0)+1 FROM roles WHERE server_id=?")
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?;
        if !owner {
            position = position.min(actor_highest.saturating_sub(1));
            if position <= 0 {
                return Err(OrganizationError::Forbidden);
            }
        }
        sqlx::query("INSERT INTO roles(id,server_id,name,color,position,permissions,is_default) VALUES(?,?,?,?,?,?,0)")
            .bind(role_id).bind(server_id).bind(name).bind(color).bind(position).bind(permission_bits)
            .execute(&mut *tx).await.map_err(OrganizationError::from)?;
        let role = sqlx::query_as("SELECT * FROM roles WHERE id=? AND server_id=?")
            .bind(role_id)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(OrganizationError::from)?;
        bump_role_projection_version(&mut tx, server_id).await?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(role)
    }

    pub async fn update_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permission_bits: i64,
    ) -> Result<RoleRow, OrganizationError> {
        let server_id = server_id.as_str();
        validate_role_fields(name, color)?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        let actor_permissions = self
            .authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        require_role_grant(actor_permissions, permission_bits)?;
        let existing: RoleRow = sqlx::query_as("SELECT * FROM roles WHERE id=? AND server_id=?")
            .bind(role_id)
            .bind(server_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(OrganizationError::from)?
            .ok_or(OrganizationError::Forbidden)?;
        let role = if existing.is_default != 0 {
            if name != existing.name || color != existing.color.as_deref() {
                return Err(OrganizationError::Forbidden);
            }
            sqlx::query_as("UPDATE roles SET permissions=? WHERE id=? AND server_id=? AND is_default=1 RETURNING *")
                .bind(permission_bits)
                .bind(role_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
        } else {
            require_managed_role(&mut tx, actor, server_id, role_id).await?;
            sqlx::query_as("UPDATE roles SET name=?,color=?,permissions=? WHERE id=? AND server_id=? AND is_default=0 RETURNING *")
                .bind(name)
                .bind(color)
                .bind(permission_bits)
                .bind(role_id)
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
        };
        bump_role_projection_version(&mut tx, server_id).await?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(role)
    }

    pub async fn delete_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        role_id: &str,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_ROLES,
            )
            .await
            .map_err(OrganizationError::from)?;
        require_managed_role(&mut tx, actor, server_id, role_id).await?;
        sqlx::query(
            "DELETE FROM channel_permission_overrides \
             WHERE target_type='role' AND target_id=? AND channel_id IN ( \
                 SELECT id FROM channels WHERE server_id=? \
             )",
        )
        .bind(role_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
        let changed = sqlx::query("DELETE FROM roles WHERE id=? AND server_id=? AND is_default=0")
            .bind(role_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await
            .map_err(OrganizationError::from)?
            .rows_affected();
        if changed != 1 {
            return Err(OrganizationError::Forbidden);
        }
        bump_role_projection_version(&mut tx, server_id).await?;
        tx.commit().await.map_err(OrganizationError::from)
    }

    pub async fn set_member_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        role_id: &str,
        assign: bool,
    ) -> Result<Vec<String>, OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(OrganizationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_ROLES,
            )
            .await
            .map_err(OrganizationError::from)?;
        require_managed_role(&mut tx, actor, server_id, role_id).await?;
        let target_member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
        )
        .bind(server_id)
        .bind(target_user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        if !target_member {
            return Err(OrganizationError::Forbidden);
        }
        let owner: bool = sqlx::query_scalar("SELECT owner_id=? FROM servers WHERE id=?")
            .bind(actor.user_id().as_str())
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(OrganizationError::from)?;
        if !owner {
            let actor_highest =
                highest_role_position(&mut tx, server_id, actor.user_id().as_str(), false).await?;
            let target_highest =
                highest_role_position(&mut tx, server_id, target_user_id, false).await?;
            if actor_highest <= target_highest {
                return Err(OrganizationError::Forbidden);
            }
        }
        let changed = if assign {
            sqlx::query("INSERT OR IGNORE INTO user_roles(server_id,user_id,role_id) VALUES(?,?,?)")
                .bind(server_id)
                .bind(target_user_id)
                .bind(role_id)
                .execute(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
                .rows_affected()
        } else {
            sqlx::query("DELETE FROM user_roles WHERE server_id=? AND user_id=? AND role_id=?")
                .bind(server_id)
                .bind(target_user_id)
                .bind(role_id)
                .execute(&mut *tx)
                .await
                .map_err(OrganizationError::from)?
                .rows_affected()
        };
        if changed != 0 {
            bump_role_projection_version(&mut tx, server_id).await?;
        }
        let roles = sqlx::query_scalar(
            "SELECT role_id FROM user_roles WHERE server_id=? AND user_id=? ORDER BY role_id",
        )
        .bind(server_id)
        .bind(target_user_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(OrganizationError::from)?;
        tx.commit().await.map_err(OrganizationError::from)?;
        Ok(roles)
    }
}
