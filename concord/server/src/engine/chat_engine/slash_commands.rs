use super::{
    ChatEngine, ChatEvent, ConnectionId, Permissions, SlashCommandInfo, SlashCommandOption, Uuid,
    validate_slash_command_options,
};

impl ChatEngine {
    /// Register a slash command for a bot. Caller must own the bot.
    pub async fn register_slash_command(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        description: &str,
        options_json: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_bot_installation_scope(auth, &actor, server_id, "commands")
            .await
            .map_err(|error| error.to_string())?;

        if name.is_empty()
            || name.len() > 32
            || !name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
        {
            return Err("Command name must be 1-32 lowercase letters, digits, '_' or '-'".into());
        }
        if description.is_empty()
            || description.len() > 100
            || description.chars().any(char::is_control)
        {
            return Err("Command description must be 1-100 printable characters".into());
        }

        let id = Uuid::new_v4().to_string();
        let opts = options_json.unwrap_or("[]");
        // Validate JSON
        let options = serde_json::from_str::<Vec<SlashCommandOption>>(opts)
            .map_err(|e| format!("Invalid options JSON: {e}"))?;
        validate_slash_command_options(&options)?;

        use crate::db::models::CreateSlashCommandParams;
        let params = CreateSlashCommandParams {
            id: &id,
            bot_user_id: actor.user_id().as_str(),
            server_id: Some(server_id),
            name,
            description,
            options_json: opts,
        };

        crate::db::queries::slash_commands::create_command(pool, &params)
            .await
            .map_err(|e| format!("Failed to register command: {e}"))?;

        let cmd = SlashCommandInfo {
            id: id.clone(),
            bot_user_id: actor.user_id().as_str().to_owned(),
            name: name.to_string(),
            description: description.to_string(),
            options,
        };

        self.broadcast_to_server(
            server_id,
            &ChatEvent::SlashCommandUpdate {
                server_id: server_id.to_string(),
                command: cmd,
            },
        );

        Ok(())
    }
    /// List slash commands available in a server.
    pub async fn list_slash_commands(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::VIEW_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::slash_commands::list_commands_for_server(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list commands: {e}"))?;

        let commands: Vec<SlashCommandInfo> = rows
            .into_iter()
            .map(|r| {
                let options: Vec<SlashCommandOption> =
                    serde_json::from_str(&r.options_json).unwrap_or_default();
                SlashCommandInfo {
                    id: r.id,
                    bot_user_id: r.bot_user_id,
                    name: r.name,
                    description: r.description,
                    options,
                }
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::SlashCommandList {
                server_id: server_id.to_string(),
                commands,
            });
        }

        Ok(())
    }
    /// Delete a slash command.
    pub async fn delete_slash_command(
        &self,
        session_id: ConnectionId,
        command_id: &str,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let cmd = crate::db::queries::slash_commands::get_command(pool, command_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Command not found")?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        // An installed command-scoped bot may remove its own registration;
        // server managers may remove any command from their server.
        if let Some(sid) = &cmd.server_id {
            if actor.user_id().as_str() == cmd.bot_user_id {
                let auth = self
                    .auth
                    .get()
                    .ok_or("Credential authority is not configured")?;
                crate::engine::authorization::AuthorizationService::new(pool.clone())
                    .authorize_bot_installation_scope(auth, &actor, sid, "commands")
                    .await
                    .map_err(|error| error.to_string())?;
            } else {
                self.require_permission(session_id, sid, None, Permissions::MANAGE_SERVER)
                    .await?;
            }
        } else {
            // Global command with no server — just verify authentication.
            let _user_id = self.get_user_id(session_id)?;
        }

        crate::db::queries::slash_commands::delete_command(pool, command_id)
            .await
            .map_err(|e| format!("Failed to delete command: {e}"))?;

        if let Some(sid) = &cmd.server_id {
            self.broadcast_to_server(
                sid,
                &ChatEvent::SlashCommandDelete {
                    server_id: sid.clone(),
                    command_id: command_id.to_string(),
                },
            );
        }

        Ok(())
    }
}
