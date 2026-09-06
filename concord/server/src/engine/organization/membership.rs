use super::{Actor, OrganizationError, OrganizationService, Permissions, ServerId};

impl OrganizationService {
    pub async fn join_server(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let admitted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM servers s WHERE s.id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id=?))",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        if !admitted {
            return Err(OrganizationError::Forbidden);
        }
        sqlx::query(
            "INSERT OR IGNORE INTO server_members(server_id,user_id,role) VALUES(?,?,'member')",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn leave_server(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(OrganizationError::Authentication)?;
        let removed = sqlx::query(
            "DELETE FROM server_members WHERE server_id=? AND user_id=? AND EXISTS(SELECT 1 FROM servers s WHERE s.id=? AND s.owner_id<>?)",
        )
        .bind(server_id).bind(actor.user_id().as_str()).bind(server_id).bind(actor.user_id().as_str())
        .execute(&mut *tx).await?;
        if removed.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_server_nickname(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        nickname: Option<&str>,
    ) -> Result<Option<String>, OrganizationError> {
        let server_id = server_id.as_str();
        if let Some(nickname) = nickname {
            crate::engine::validation::validate_display_name(nickname)
                .map_err(|_| OrganizationError::InvalidInput("invalid server nickname"))?;
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .server_actor_permissions_in(&mut tx, &self.auth, actor, server_id)
            .await
            .map_err(OrganizationError::from)?;
        let updated =
            sqlx::query("UPDATE server_members SET nickname=? WHERE server_id=? AND user_id=?")
                .bind(nickname.map(str::trim))
                .bind(server_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        let avatar = sqlx::query_scalar(
            "SELECT avatar_url FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(avatar)
    }

    pub async fn update_server(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(String, Option<String>), OrganizationError> {
        let server_id = server_id.as_str();
        if let Some(name) = name {
            crate::engine::validation::validate_server_name(name)
                .map_err(|_| OrganizationError::InvalidInput("invalid server name"))?;
        }
        if icon_url.is_some_and(|url| url.len() > 2_000 || url.chars().any(char::is_control)) {
            return Err(OrganizationError::InvalidInput("invalid server icon"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(OrganizationError::from)?;
        let current: (String, Option<String>) =
            sqlx::query_as("SELECT name,icon_url FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await?;
        let next_name = name.unwrap_or(&current.0).to_owned();
        let next_icon = icon_url.map(str::to_owned).or(current.1);
        sqlx::query("UPDATE servers SET name=?,icon_url=?,updated_at=datetime('now') WHERE id=?")
            .bind(&next_name)
            .bind(&next_icon)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok((next_name, next_icon))
    }

    pub async fn update_emoji_settings(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        allow_external: bool,
        shareable: bool,
    ) -> Result<(), OrganizationError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        sqlx::query("UPDATE servers SET allow_external_emoji=?,shareable_emoji=?,updated_at=datetime('now') WHERE id=?")
            .bind(i64::from(allow_external)).bind(i64::from(shareable)).bind(server_id)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_member_role(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        role: &str,
    ) -> Result<(), OrganizationError> {
        use crate::engine::permissions::ServerRole;

        let desired = match role {
            "admin" => ServerRole::Admin,
            "moderator" => ServerRole::Moderator,
            "member" => ServerRole::Member,
            _ => return Err(OrganizationError::InvalidInput("invalid server role")),
        };
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_ROLES,
            )
            .await?;
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT caller.role,target.role FROM server_members caller \
             JOIN server_members target ON target.server_id=caller.server_id \
             JOIN servers s ON s.id=caller.server_id \
             WHERE caller.server_id=? AND caller.user_id=? AND target.user_id=? \
               AND target.user_id<>s.owner_id",
        )
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .bind(target_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OrganizationError::Forbidden)?;
        let caller = ServerRole::parse(&row.0);
        let current = ServerRole::parse(&row.1);
        if !caller.can_manage_roles(&current) || !caller.can_manage_roles(&desired) {
            return Err(OrganizationError::Forbidden);
        }
        let updated =
            sqlx::query("UPDATE server_members SET role=? WHERE server_id=? AND user_id=?")
                .bind(desired.as_str())
                .bind(server_id)
                .bind(target_user_id)
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_member_avatar(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        avatar_url: Option<&str>,
    ) -> Result<(), OrganizationError> {
        if avatar_url.is_some_and(|url| url.len() > 2_000 || url.chars().any(char::is_control)) {
            return Err(OrganizationError::InvalidInput("invalid server avatar"));
        }
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let updated =
            sqlx::query("UPDATE server_members SET avatar_url=? WHERE server_id=? AND user_id=?")
                .bind(avatar_url)
                .bind(server_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(OrganizationError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }
}
