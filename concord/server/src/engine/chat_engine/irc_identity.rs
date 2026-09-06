use super::ChatEngine;

impl ChatEngine {
    /// Resolve an IRC channel through the actor's durable default server and
    /// server/channel aliases, then authorize the resolved stable channel ID.
    pub async fn resolve_irc_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_alias: Option<&str>,
    ) -> Result<String, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        auth.validate_actor(actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let server_id = match server_alias {
            Some(alias) => {
                crate::db::queries::aliases::resolve_server_alias(
                    pool,
                    alias.trim_start_matches('#'),
                    actor.user_id().as_str(),
                )
                .await
            }
            None => {
                crate::db::queries::aliases::get_default_server(pool, actor.user_id().as_str())
                    .await
            }
        }
        .map_err(|_| "resource unavailable".to_string())?
        .ok_or_else(|| "resource unavailable".to_string())?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .server_members_for_actor(auth, actor, &server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        Ok(server_id)
    }
    pub async fn resolve_irc_channel_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        irc_name: &str,
    ) -> Result<(String, String), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let bare = irc_name.strip_prefix('#').unwrap_or(irc_name);
        let (server_id, channel_alias) =
            if let Some((server_alias, channel_alias)) = bare.split_once('/') {
                let server_id = self
                    .resolve_irc_server_for_actor(actor, Some(server_alias))
                    .await?;
                (server_id, channel_alias)
            } else {
                let server_id = self.resolve_irc_server_for_actor(actor, None).await?;
                (server_id, bare)
            };
        let channel_id =
            crate::db::queries::aliases::resolve_channel_alias(pool, &server_id, channel_alias)
                .await
                .map_err(|_| "resource unavailable".to_string())?
                .ok_or_else(|| "resource unavailable".to_string())?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor(
                auth,
                actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::View,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let channel_name: String =
            sqlx::query_scalar("SELECT name FROM channels WHERE id=? AND server_id=?")
                .bind(channel_id)
                .bind(&server_id)
                .fetch_one(pool)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        Ok((server_id, channel_name))
    }
    /// Get the owner user ID for a server.
    pub fn get_server_owner_id(&self, server_id: &str) -> Option<String> {
        self.servers.get(server_id).map(|s| s.owner_id.clone())
    }
    /// Get IRC-style mode string for a channel (e.g., "+ins").
    pub fn get_channel_modes(&self, server_id: &str, channel_name: &str) -> String {
        let key = (server_id.to_string(), channel_name.to_string());
        let Some(channel_id) = self.channel_name_index.get(&key).map(|v| v.clone()) else {
            return "+".to_string();
        };
        let Some(ch) = self.channels.get(&channel_id) else {
            return "+".to_string();
        };
        let mut modes = String::from("+n"); // no external messages (always set)
        if ch.is_private {
            modes.push('i'); // invite-only
        }
        if ch.slowmode_seconds > 0 {
            modes.push('m'); // moderated (closest IRC analog to slowmode)
        }
        modes
    }
}
