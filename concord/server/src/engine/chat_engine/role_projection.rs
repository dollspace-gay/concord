use super::{
    ChatEngine, ChatEvent, ConnectionId, Permissions, referenced_channel_id, referenced_server_id,
    role_row_to_info,
};

impl ChatEngine {
    /// Assign a role to a user. Enforces role hierarchy.
    pub async fn assign_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, String> {
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
        .set_member_role(
            &actor,
            &referenced_server_id(server_id)?,
            target_user_id,
            role_id,
            true,
        )
        .await
        .map_err(Into::into)
    }
    /// Remove a role from a user. Enforces role hierarchy.
    pub async fn remove_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, String> {
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
        .set_member_role(
            &actor,
            &referenced_server_id(server_id)?,
            target_user_id,
            role_id,
            false,
        )
        .await
        .map_err(Into::into)
    }
    /// Publish one authoritative role projection while holding write admission,
    /// preventing an older post-commit notification from overtaking a newer edit.
    pub async fn broadcast_role_snapshot(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        changed_user_id: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let (_permit, mut tx) = writes.begin().await.map_err(|error| error.to_string())?;
        crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        )
        .server_actor_permissions_in(&mut tx, auth, &actor, server_id)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let version: i64 =
            sqlx::query_scalar("SELECT role_projection_version FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| "resource unavailable")?;
        let roles: Vec<crate::db::models::RoleRow> =
            sqlx::query_as("SELECT * FROM roles WHERE server_id=? ORDER BY position DESC")
                .bind(server_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        let role_ids = if let Some(user_id) = changed_user_id {
            Some(sqlx::query_scalar(
                "SELECT role_id FROM user_roles WHERE server_id=? AND user_id=? ORDER BY role_id",
            ).bind(server_id).bind(user_id).fetch_all(&mut *tx).await
                .map_err(|_| "resource unavailable".to_string())?)
        } else {
            None
        };
        tx.commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        self.broadcast_to_server(
            server_id,
            &ChatEvent::RoleList {
                server_id: server_id.to_owned(),
                version,
                roles: roles.into_iter().map(role_row_to_info).collect(),
                member_roles: None,
            },
        );
        if let (Some(user_id), Some(role_ids)) = (changed_user_id, role_ids) {
            self.broadcast_to_server(
                server_id,
                &ChatEvent::MemberRoleUpdate {
                    server_id: server_id.to_owned(),
                    version,
                    user_id: user_id.to_owned(),
                    role_ids,
                },
            );
        }
        Ok(())
    }
    pub async fn list_channel_permission_overrides(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
    ) -> Result<Vec<crate::engine::events::ChannelPermissionOverrideInfo>, String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        self.organization_service()?
            .list_channel_overrides(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(channel_id)?,
            )
            .await
            .map_err(Into::into)
    }
    pub async fn set_channel_permission_override(
        &self,
        session_id: ConnectionId,
        update: crate::engine::organization::ChannelOverrideUpdate<'_>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        self.organization_service()?
            .set_channel_override(&actor, update)
            .await
            .map_err(Into::into)
    }
    pub async fn delete_channel_permission_override(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
        target_type: &str,
        target_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        self.organization_service()?
            .delete_channel_override(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(channel_id)?,
                target_type,
                target_id,
            )
            .await
            .map_err(Into::into)
    }
    pub async fn broadcast_channel_permission_overrides(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut tx) = writes.begin().await.map_err(|error| error.to_string())?;
        crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        )
        .require_server_actor_in(
            &mut tx,
            self.auth.get().ok_or("Authentication unavailable")?,
            &actor,
            server_id,
            Permissions::MANAGE_CHANNELS,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let rows: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT id,channel_id,target_type,target_id,allow_bits,deny_bits \
             FROM channel_permission_overrides WHERE channel_id=? \
             ORDER BY target_type,target_id",
        )
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        tx.commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let overrides = rows
            .into_iter()
            .map(
                |(id, channel_id, target_type, target_id, allow_bits, deny_bits)| {
                    crate::engine::events::ChannelPermissionOverrideInfo {
                        id,
                        channel_id,
                        target_type,
                        target_id,
                        allow_bits,
                        deny_bits,
                    }
                },
            )
            .collect();
        let event = ChatEvent::ChannelPermissionOverrideList {
            server_id: server_id.to_owned(),
            channel_id: channel_id.to_owned(),
            overrides,
        };
        let Some(server) = self.servers.get(server_id) else {
            return Ok(());
        };
        let member_ids: Vec<String> = server.member_user_ids.iter().cloned().collect();
        drop(server);
        for session in self.sessions.iter() {
            if session
                .user_id
                .as_ref()
                .is_some_and(|user_id| member_ids.contains(user_id))
            {
                let _ = session.send_guarded(
                    event.clone(),
                    Some(
                        crate::engine::user_session::DeliveryGuard::ServerPermissions(vec![(
                            server_id.to_owned(),
                            Permissions::MANAGE_CHANNELS,
                        )]),
                    ),
                );
            }
        }
        Ok(())
    }
}
