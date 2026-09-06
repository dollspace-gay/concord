use super::{
    ChatEngine, ConnectionId, MemberRoleInfo, Permissions, RoleInfo, Uuid, error,
    group_member_roles, referenced_server_id, role_row_to_info,
};

impl ChatEngine {
    /// Get effective permissions for a user in a channel.
    pub async fn get_effective_permissions(
        &self,
        server_id: &str,
        channel_id: Option<&str>,
        user_id: &str,
    ) -> Permissions {
        let Some(pool) = &self.db else {
            return Permissions::empty();
        };
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .effective_permissions(user_id, server_id, channel_id)
            .await
            .unwrap_or_else(|error| {
                error!(%error, "authorization failed closed");
                Permissions::empty()
            })
    }
    /// Check that a user has a required permission. Returns Ok(user_id) or Err(message).
    pub async fn require_permission(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: Option<&str>,
        required: Permissions,
    ) -> Result<String, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();

        let perms = self
            .get_effective_permissions(server_id, channel_id, &user_id)
            .await;

        if perms.contains(required) {
            Ok(user_id)
        } else {
            Err("FORBIDDEN: insufficient permissions".into())
        }
    }
    /// List roles for a server.
    pub async fn list_roles(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(i64, Vec<RoleInfo>, Vec<MemberRoleInfo>), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        crate::engine::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .list_roles(&actor, &referenced_server_id(server_id)?)
        .await?;
        let mut connection = pool.acquire().await.map_err(|_| "resource unavailable")?;
        let version: i64 =
            sqlx::query_scalar("SELECT role_projection_version FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *connection)
                .await
                .map_err(|_| "resource unavailable")?;
        let roles: Vec<crate::db::models::RoleRow> =
            sqlx::query_as("SELECT * FROM roles WHERE server_id=? ORDER BY position DESC")
                .bind(server_id)
                .fetch_all(&mut *connection)
                .await
                .map_err(|_| "resource unavailable")?;
        let assignments: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT sm.user_id,ur.role_id FROM server_members sm LEFT JOIN user_roles ur ON ur.server_id=sm.server_id AND ur.user_id=sm.user_id WHERE sm.server_id=? ORDER BY sm.user_id,ur.role_id",
        )
        .bind(server_id)
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| "resource unavailable")?;
        Ok((
            version,
            roles.into_iter().map(role_row_to_info).collect(),
            group_member_roles(assignments),
        ))
    }
    /// Create a custom role in a server.
    pub async fn create_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: i64,
    ) -> Result<RoleInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let role_id = Uuid::new_v4().to_string();
        let role = crate::engine::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .create_role(
            &actor,
            &referenced_server_id(server_id)?,
            &role_id,
            name,
            color,
            permissions,
        )
        .await?;

        Ok(role_row_to_info(role))
    }
    /// Update a custom role.
    pub async fn update_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: i64,
    ) -> Result<RoleInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let role = crate::engine::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .update_role(
            &actor,
            &referenced_server_id(server_id)?,
            role_id,
            name,
            color,
            permissions,
        )
        .await?;
        Ok(role_row_to_info(role))
    }
    /// Delete a custom role.
    pub async fn delete_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        role_id: &str,
    ) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        crate::engine::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .delete_role(&actor, &referenced_server_id(server_id)?, role_id)
        .await
        .map_err(Into::into)
    }
    /// Get the highest role position for a user in a server.
    /// Server owner gets i32::MAX. Returns 0 if no roles found (base @everyone level).
    pub async fn get_user_highest_role_position(&self, server_id: &str, user_id: &str) -> i32 {
        if self.is_server_owner(server_id, user_id) {
            return i32::MAX;
        }
        let Some(pool) = &self.db else { return 0 };
        crate::db::queries::roles::get_user_roles(pool, server_id, user_id)
            .await
            .unwrap_or_default()
            .iter()
            .map(|r| r.position)
            .max()
            .unwrap_or(0)
    }
    /// Validate role hierarchy: actor must have a higher position than target role.
    pub async fn check_role_hierarchy(
        &self,
        server_id: &str,
        actor_user_id: &str,
        target_role_id: &str,
    ) -> Result<(), String> {
        // Server owner bypasses hierarchy checks
        if self.is_server_owner(server_id, actor_user_id) {
            return Ok(());
        }
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let target_role = crate::db::queries::roles::get_role(pool, target_role_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Role not found")?;
        let actor_highest = self
            .get_user_highest_role_position(server_id, actor_user_id)
            .await;
        if actor_highest <= target_role.position {
            return Err("You cannot manage a role at or above your own highest role".to_string());
        }
        Ok(())
    }
    pub(super) fn evict_user_from_server_subscriptions(&self, server_id: &str, user_id: &str) {
        let sessions: std::collections::HashSet<_> = self
            .sessions
            .iter()
            .filter(|session| session.user_id.as_deref() == Some(user_id))
            .map(|session| *session.key())
            .collect();
        for mut channel in self.channels.iter_mut() {
            if channel.server_id == server_id {
                channel
                    .members
                    .retain(|session_id| !sessions.contains(session_id));
            }
        }
    }
}
