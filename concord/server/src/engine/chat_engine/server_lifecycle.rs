use super::{
    ChannelState, ChatEngine, ServerState, Uuid, info, referenced_channel_id, referenced_server_id,
    stable_irc_alias, warn,
};
use crate::engine::validation;

impl ChatEngine {
    /// Create a new server. Returns the server ID.
    pub async fn create_server(
        &self,
        name: String,
        owner_user_id: String,
        icon_url: Option<String>,
    ) -> Result<String, String> {
        validation::validate_server_name(&name)?;

        let server_id = Uuid::new_v4().to_string();
        let channel_id = Uuid::new_v4().to_string();
        let server_alias = stable_irc_alias(&name, &server_id);

        if self.db.is_some() {
            return Err("authenticated actor required".into());
        } else {
            let owned_count = self
                .servers
                .iter()
                .filter(|server| server.owner_id == owner_user_id)
                .count();
            if owned_count >= 100 {
                return Err("server limit reached".to_string());
            }
        }

        let mut state = ServerState::new(
            server_id.clone(),
            name.clone(),
            owner_user_id.clone(),
            icon_url,
        );
        state.member_user_ids.insert(owner_user_id.clone());
        self.servers.insert(server_id.clone(), state);
        self.server_alias_index
            .insert(server_alias.clone(), server_id.clone());
        self.server_aliases.insert(server_id.clone(), server_alias);

        // Mirror the committed default channel in the runtime cache.
        let channel_name = "#general".to_string();
        let ch = ChannelState::new(channel_id.clone(), server_id.clone(), channel_name.clone());
        self.channel_name_index
            .insert((server_id.clone(), channel_name), channel_id.clone());
        if let Some(mut srv) = self.servers.get_mut(&server_id) {
            srv.channel_ids.insert(channel_id.clone());
        }
        self.channels.insert(channel_id, ch);

        info!(%server_id, %name, "server created");
        Ok(server_id)
    }
    pub async fn create_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        name: String,
        icon_url: Option<String>,
    ) -> Result<String, String> {
        validation::validate_server_name(&name)?;
        let owner_user_id = actor.user_id().as_str().to_owned();
        let server_id = Uuid::new_v4().to_string();
        let channel_id = Uuid::new_v4().to_string();
        let server_alias = stable_irc_alias(&name, &server_id);
        self.organization_service()?
            .provision_server(
                actor,
                &name,
                icon_url.as_deref(),
                &referenced_server_id(&server_id)?,
                &referenced_channel_id(&channel_id)?,
                &server_alias,
            )
            .await
            .map_err(String::from)?;
        let mut state = ServerState::new(server_id.clone(), name, owner_user_id.clone(), icon_url);
        state.member_user_ids.insert(owner_user_id);
        self.servers.insert(server_id.clone(), state);
        self.server_alias_index
            .insert(server_alias.clone(), server_id.clone());
        self.server_aliases.insert(server_id.clone(), server_alias);
        let channel_name = "#general".to_string();
        let channel =
            ChannelState::new(channel_id.clone(), server_id.clone(), channel_name.clone());
        self.channel_name_index
            .insert((server_id.clone(), channel_name), channel_id.clone());
        if let Some(mut server) = self.servers.get_mut(&server_id) {
            server.channel_ids.insert(channel_id.clone());
        }
        self.channels.insert(channel_id, channel);
        Ok(server_id)
    }
    /// Delete a server.
    pub async fn delete_server(&self, server_id: &str) -> Result<(), String> {
        if let Some(pool) = &self.db {
            crate::db::queries::servers::delete_server(pool, server_id)
                .await
                .map_err(|e| format!("Failed to delete server: {e}"))?;
        }

        self.remove_server_from_cache(server_id);

        info!(%server_id, "server deleted");
        Ok(())
    }
    pub async fn admin_delete_server(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .admin_delete_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        self.remove_server_from_cache(server_id);
        Ok(())
    }
    pub async fn delete_owned_server(
        &self,
        server_id: &str,
        actor: &crate::auth::authority::Actor,
    ) -> Result<(), String> {
        self.organization_service()?
            .delete_owned_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        self.remove_server_from_cache(server_id);
        info!(%server_id, "server deleted");
        Ok(())
    }
    pub(super) fn remove_server_from_cache(&self, server_id: &str) {
        if let Some(server) = self.servers.get(server_id) {
            for ch_id in &server.channel_ids {
                if let Some((_, ch)) = self.channels.remove(ch_id) {
                    self.channel_name_index
                        .remove(&(server_id.to_string(), ch.name));
                }
            }
        }
        if let Some((_, alias)) = self.server_aliases.remove(server_id) {
            self.server_alias_index.remove(&alias);
        }
        self.servers.remove(server_id);
    }
    /// Update a server's name and/or icon.
    pub async fn update_server_settings(
        &self,
        server_id: &str,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(), String> {
        // Compute new values and apply in-memory update while holding the guard,
        // then drop the guard before any .await to avoid holding the DashMap shard
        // lock across an async suspension point.
        let (new_name, new_icon) = {
            let mut server = self
                .servers
                .get_mut(server_id)
                .ok_or_else(|| format!("No such server: {server_id}"))?;

            let new_name = name.unwrap_or(&server.name).to_string();
            let new_icon = if icon_url.is_some() {
                icon_url.map(|s| s.to_string())
            } else {
                server.icon_url.clone()
            };

            server.name = new_name.clone();
            server.icon_url = new_icon.clone();

            (new_name, new_icon)
        }; // guard dropped here

        if let Some(pool) = &self.db
            && let Err(e) = crate::db::queries::servers::update_server(
                pool,
                server_id,
                &new_name,
                new_icon.as_deref(),
            )
            .await
        {
            warn!(%server_id, error = %e, "failed to persist server settings update to DB");
            return Err(format!("Failed to update server: {e}"));
        }

        info!(%server_id, "server settings updated");
        Ok(())
    }
    pub async fn update_server_settings_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(), String> {
        let (new_name, new_icon) = self
            .organization_service()?
            .update_server(actor, &referenced_server_id(server_id)?, name, icon_url)
            .await
            .map_err(String::from)?;
        let mut server = self
            .servers
            .get_mut(server_id)
            .ok_or("FORBIDDEN: resource unavailable")?;
        server.name = new_name;
        server.icon_url = new_icon;
        Ok(())
    }
    pub async fn update_emoji_settings_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        allow_external: bool,
        shareable: bool,
    ) -> Result<(), String> {
        self.organization_service()?
            .update_emoji_settings(
                actor,
                &referenced_server_id(server_id)?,
                allow_external,
                shareable,
            )
            .await
            .map_err(String::from)
    }
}
