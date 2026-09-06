use super::{ChatEngine, ChatEvent, ServerInfo, ServerRole, referenced_server_id};

impl ChatEngine {
    pub async fn update_member_role_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        target_user_id: &str,
        role: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .update_member_role(
                actor,
                &referenced_server_id(server_id)?,
                target_user_id,
                role,
            )
            .await
            .map_err(String::from)
    }
    pub async fn set_member_avatar_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), String> {
        self.organization_service()?
            .set_member_avatar(actor, &referenced_server_id(server_id)?, avatar_url)
            .await
            .map_err(String::from)?;
        self.broadcast_to_server(
            server_id,
            &ChatEvent::ServerAvatarUpdate {
                server_id: server_id.to_owned(),
                user_id: actor.user_id().as_str().to_owned(),
                avatar_url: avatar_url.map(str::to_owned),
            },
        );
        Ok(())
    }
    /// List servers for a user (by their DB user_id).
    pub async fn list_servers_for_user(&self, user_id: &str) -> Vec<ServerInfo> {
        let mut servers = Vec::new();
        for entry in self.servers.iter() {
            let s = entry.value();
            if !s.member_user_ids.contains(user_id) {
                continue;
            }
            let role = if s.owner_id == user_id {
                Some("owner".to_string())
            } else {
                Some("member".to_string())
            };
            let perms = self.get_effective_permissions(&s.id, None, user_id).await;
            servers.push(ServerInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                icon_url: s.icon_url.clone(),
                member_count: s.member_user_ids.len(),
                role,
                my_permissions: perms.bits() as i64,
            });
        }
        servers
    }
    pub async fn list_servers_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<ServerInfo>, String> {
        self.organization_service()?
            .list_servers_for_actor(actor)
            .await
            .map_err(String::from)
    }
    pub async fn server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(ServerInfo, crate::engine::authorization::AuthorizationStamp), String> {
        self.organization_service()?
            .server_for_actor(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)
    }
    pub async fn server_members_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<crate::engine::organization::ServerMemberSummary>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.organization_service()?
            .server_members_for_actor(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)
    }
    pub async fn list_all_servers_for_admin(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<ServerInfo>, String> {
        self.organization_service()?
            .list_servers_as_admin(actor)
            .await
            .map_err(String::from)
    }
    pub async fn set_system_admin_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        target_user_id: &str,
        is_admin: bool,
    ) -> Result<(), String> {
        self.organization_service()?
            .set_system_admin(actor, target_user_id, is_admin)
            .await
            .map_err(String::from)
    }
    /// List all servers (for system admin).
    pub fn list_all_servers(&self) -> Vec<ServerInfo> {
        self.servers
            .iter()
            .map(|s| ServerInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                icon_url: s.icon_url.clone(),
                member_count: s.member_user_ids.len(),
                role: None,
                my_permissions: 0,
            })
            .collect()
    }
    /// Check if a user is the owner of a server.
    pub fn is_server_owner(&self, server_id: &str, user_id: &str) -> bool {
        self.servers
            .get(server_id)
            .map(|s| s.owner_id == user_id)
            .unwrap_or(false)
    }
    /// Check if a user is a member of a server (in-memory check).
    pub fn user_is_server_member(&self, server_id: &str, user_id: &str) -> bool {
        self.servers
            .get(server_id)
            .map(|s| s.member_user_ids.contains(user_id))
            .unwrap_or(false)
    }
    /// Join a server (persistent membership).
    pub async fn join_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if !self.servers.contains_key(server_id) {
            return Err(format!("No such server: {server_id}"));
        }

        // Check if the user is banned from this server
        if let Some(pool) = &self.db {
            if crate::db::queries::bans::is_banned(pool, server_id, user_id)
                .await
                .unwrap_or(false)
            {
                return Err("You are banned from this server".into());
            }

            crate::db::queries::servers::add_server_member(pool, server_id, user_id, "member")
                .await
                .map_err(|e| format!("Failed to join server: {e}"))?;
        }

        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.insert(user_id.to_string());
        }

        Ok(())
    }
    pub async fn join_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .join_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server
                .member_user_ids
                .insert(actor.user_id().as_str().to_owned());
        }
        Ok(())
    }
    /// Leave a server (remove persistent membership).
    pub async fn leave_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if let Some(pool) = &self.db {
            crate::db::queries::servers::remove_server_member(pool, server_id, user_id)
                .await
                .map_err(|e| format!("Failed to leave server: {e}"))?;
        }

        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(user_id);
        }

        Ok(())
    }
    pub async fn leave_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .leave_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(actor.user_id().as_str());
        }
        Ok(())
    }
    /// Get the role of a user in a server.
    pub async fn get_server_role(&self, server_id: &str, user_id: &str) -> Option<ServerRole> {
        let Some(pool) = &self.db else {
            return None;
        };
        let member = crate::db::queries::servers::get_server_member(pool, server_id, user_id)
            .await
            .ok()
            .flatten()?;
        Some(ServerRole::parse(&member.role))
    }
    /// Look up server_id by server name (for IRC).
    pub fn find_server_by_name(&self, name: &str) -> Option<String> {
        let name_lower = name.to_lowercase();
        if let Some(server_id) = self.server_alias_index.get(&name_lower) {
            return Some(server_id.clone());
        }
        self.servers
            .iter()
            .find(|s| s.name.to_lowercase() == name_lower)
            .map(|s| s.id.clone())
    }
    /// Get a server's name by ID.
    pub fn get_server_name(&self, server_id: &str) -> Option<String> {
        self.servers.get(server_id).map(|s| s.name.clone())
    }
    pub fn get_server_alias(&self, server_id: &str) -> Option<String> {
        self.server_aliases
            .get(server_id)
            .map(|alias| alias.clone())
    }
}
