use super::{
    ChatEngine, ChatEvent, ConnectionId, InteractionInfo, SlashCommandOption, Utc, Uuid,
    normalize_channel_name, validate_slash_command_arguments,
};

impl ChatEngine {
    /// Invoke a slash command. Creates an interaction and dispatches to the bot.
    pub async fn invoke_slash_command(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel: &str,
        command_name: &str,
        args_json: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let user_id = actor.user_id().as_str().to_owned();

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Resolve channel_id from name (normalize for case-insensitive lookup)
        let channel = normalize_channel_name(channel);
        let channel_id = self.resolve_channel_id(server_id, &channel)?;
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let (_permit, mut transaction) = self.begin_admitted_write().await?;
        let cmd = sqlx::query_as::<_, crate::db::models::SlashCommandRow>(
            "SELECT c.* FROM slash_commands c \
             JOIN bot_installations i ON i.bot_user_id=c.bot_user_id AND i.server_id=? \
             WHERE (c.server_id=? OR c.server_id IS NULL) AND c.name=? COLLATE NOCASE \
               AND i.state='active' AND i.revoked_at IS NULL \
               AND (instr(' '||i.granted_scopes||' ',' commands ')>0 \
                    OR instr(' '||i.granted_scopes||' ',' * ')>0)",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(command_name)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?
        .ok_or_else(|| format!("NOT_FOUND: unknown command /{command_name}"))?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                &actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::Send,
            )
            .await
            .map_err(|error| error.to_string())?;

        let interaction_id = Uuid::new_v4().to_string();
        let data: serde_json::Value = match args_json {
            Some(value) if value.len() <= 8 * 1024 => serde_json::from_str(value)
                .map_err(|_| "Command arguments must be valid JSON".to_string())?,
            Some(_) => return Err("Command arguments are too large".into()),
            None => serde_json::Value::Object(serde_json::Map::new()),
        };
        if !data.is_object() {
            return Err("Command arguments must be a JSON object".into());
        }
        let options: Vec<SlashCommandOption> = serde_json::from_str(&cmd.options_json)
            .map_err(|_| "Command definition is unavailable".to_string())?;
        validate_slash_command_arguments(&options, &data)?;
        let arguments = data.as_object().expect("argument object was validated");
        for option in &options {
            let Some(value) = arguments
                .get(&option.name)
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let exists = match option.option_type.as_str() {
                "user" => sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
                )
                .bind(server_id)
                .bind(value)
                .fetch_one(&mut *transaction)
                .await,
                "channel" => {
                    sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(SELECT 1 FROM channels WHERE server_id=? AND id=?)",
                    )
                    .bind(server_id)
                    .bind(value)
                    .fetch_one(&mut *transaction)
                    .await
                }
                "role" => {
                    sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(SELECT 1 FROM roles WHERE server_id=? AND id=?)",
                    )
                    .bind(server_id)
                    .bind(value)
                    .fetch_one(&mut *transaction)
                    .await
                }
                _ => continue,
            }
            .map_err(|error| format!("DB error: {error}"))?;
            if !exists {
                return Err(format!("Unknown value for command option: {}", option.name));
            }
        }

        let data_str = serde_json::to_string(&data).unwrap_or_default();
        let interaction_expires_at = (Utc::now() + chrono::Duration::minutes(15))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let interaction_params = crate::db::models::CreateInteractionParams {
            id: &interaction_id,
            interaction_type: "slash_command",
            command_id: Some(&cmd.id),
            user_id: &user_id,
            server_id,
            channel_id: &channel_id,
            data_json: &data_str,
            application_user_id: &cmd.bot_user_id,
            expires_at: &interaction_expires_at,
        };
        sqlx::query(
            "INSERT INTO interactions \
             (id,interaction_type,command_id,user_id,server_id,channel_id,data_json, \
              application_user_id,expires_at,response_state) \
             VALUES(?, 'slash_command', ?, ?, ?, ?, ?, ?, ?, 'pending')",
        )
        .bind(interaction_params.id)
        .bind(interaction_params.command_id)
        .bind(interaction_params.user_id)
        .bind(interaction_params.server_id)
        .bind(interaction_params.channel_id)
        .bind(interaction_params.data_json)
        .bind(interaction_params.application_user_id)
        .bind(interaction_params.expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;
        transaction
            .commit()
            .await
            .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;

        let interaction = InteractionInfo {
            id: interaction_id.clone(),
            interaction_type: "slash_command".to_string(),
            command_name: Some(command_name.to_string()),
            user_id: user_id.clone(),
            server_id: server_id.to_string(),
            channel_id: channel_id.clone(),
            data,
        };

        // Send to the bot (find sessions for the bot user)
        for entry in self.sessions.iter() {
            let s = entry.value();
            if let Some(ref uid) = s.user_id
                && uid == &cmd.bot_user_id
            {
                let _ = s.send_guarded(
                    ChatEvent::InteractionCreate {
                        interaction: interaction.clone(),
                    },
                    Some(
                        crate::engine::user_session::DeliveryGuard::BotInstallationScopes(vec![(
                            server_id.to_owned(),
                            "commands".to_owned(),
                        )]),
                    ),
                );
            }
        }

        // Also send a notice to the invoker
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerNotice {
                message: format!("/{command_name} invoked"),
            });
        }

        Ok(())
    }
}
