use super::{
    ChannelAction, ChatEngine, ChatEvent, ConnectionId, InteractionInfo, InteractionResponseData,
    Utc, Uuid, find_message_component,
};

impl ChatEngine {
    /// Invoke a button or select menu from a persisted bot response.
    pub async fn invoke_message_component(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        custom_id: &str,
        values: &[String],
    ) -> Result<(), String> {
        if message_id.is_empty()
            || message_id.len() > 128
            || custom_id.is_empty()
            || custom_id.len() > 100
            || values.len() > 25
            || values.iter().any(|value| value.len() > 100)
        {
            return Err("Invalid message component invocation".into());
        }
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let (_permit, mut transaction) = self.begin_admitted_write().await?;
        use sqlx::Row;
        let (server_id, channel_id, application_user_id, components): (
            String,
            String,
            String,
            Vec<crate::engine::events::MessageComponent>,
        ) = if let Some(interaction_id) = message_id.strip_prefix("ephemeral:") {
            let row = sqlx::query(
                "SELECT server_id,channel_id,application_user_id,ephemeral_response_json \
                     FROM interactions WHERE id=? AND user_id=? AND response_state='responded' \
                       AND response_expires_at IS NOT NULL \
                       AND response_expires_at>datetime('now')",
            )
            .bind(interaction_id)
            .bind(actor.user_id().as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| format!("DB error: {error}"))?
            .ok_or("Message component not found")?;
            let response: InteractionResponseData = serde_json::from_str(row.get::<&str, _>(3))
                .map_err(|_| "Message component is unavailable".to_string())?;
            (
                row.get(0),
                row.get(1),
                row.get(2),
                response.components.unwrap_or_default(),
            )
        } else {
            let row = sqlx::query(
                "SELECT m.server_id,m.channel_id,m.components_json,i.application_user_id \
                     FROM messages m JOIN interactions i ON i.response_message_id=m.id \
                     WHERE m.id=? AND m.deleted_at IS NULL",
            )
            .bind(message_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| format!("DB error: {error}"))?
            .ok_or("Message component not found")?;
            let components = serde_json::from_str(row.get::<&str, _>(2))
                .map_err(|_| "Message component is unavailable".to_string())?;
            (row.get(0), row.get(1), row.get(3), components)
        };
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                &actor,
                &channel_id,
                ChannelAction::View,
            )
            .await
            .map_err(|error| error.to_string())?;
        let component =
            find_message_component(&components, custom_id).ok_or("Message component not found")?;
        let interaction_type = match component {
            crate::engine::events::MessageComponent::Button { disabled, .. } => {
                if *disabled || !values.is_empty() {
                    return Err("Message component is unavailable".into());
                }
                "button"
            }
            crate::engine::events::MessageComponent::SelectMenu {
                options,
                min_values,
                max_values,
                ..
            } => {
                let distinct: std::collections::HashSet<_> = values.iter().collect();
                if *min_values < 0
                    || *max_values < *min_values
                    || values.len() < *min_values as usize
                    || values.len() > *max_values as usize
                    || distinct.len() != values.len()
                {
                    return Err("Invalid select menu values".into());
                }
                if values
                    .iter()
                    .any(|value| !options.iter().any(|option| option.value == *value))
                {
                    return Err("Invalid select menu values".into());
                }
                "select_menu"
            }
            crate::engine::events::MessageComponent::ActionRow { .. } => {
                return Err("Message component not found".into());
            }
        };
        let installation_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM bot_installations \
             WHERE bot_user_id=? AND server_id=? AND state='active' AND revoked_at IS NULL \
             AND (instr(' '||granted_scopes||' ',' commands ')>0 \
                  OR instr(' '||granted_scopes||' ',' * ')>0))",
        )
        .bind(&application_user_id)
        .bind(&server_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| format!("DB error: {error}"))?;
        if !installation_active {
            return Err("Message component is unavailable".into());
        }
        let interaction_id = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() + chrono::Duration::minutes(15))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let data = serde_json::json!({
            "message_id": message_id,
            "custom_id": custom_id,
            "values": values,
        });
        let data_json = serde_json::to_string(&data)
            .map_err(|_| "Invalid message component invocation".to_string())?;
        sqlx::query(
            "INSERT INTO interactions \
             (id,interaction_type,user_id,server_id,channel_id,data_json, \
              application_user_id,expires_at,response_state) \
             VALUES(?,?,?,?,?,?,?,?, 'pending')",
        )
        .bind(&interaction_id)
        .bind(interaction_type)
        .bind(actor.user_id().as_str())
        .bind(&server_id)
        .bind(&channel_id)
        .bind(&data_json)
        .bind(&application_user_id)
        .bind(&expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;
        transaction
            .commit()
            .await
            .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;
        let interaction = InteractionInfo {
            id: interaction_id,
            interaction_type: interaction_type.to_owned(),
            command_name: None,
            user_id: actor.user_id().as_str().to_owned(),
            server_id: server_id.clone(),
            channel_id,
            data,
        };
        for entry in self.sessions.iter() {
            let session = entry.value();
            if session.user_id.as_deref() == Some(application_user_id.as_str()) {
                let _ = session.send_guarded(
                    ChatEvent::InteractionCreate {
                        interaction: interaction.clone(),
                    },
                    Some(
                        crate::engine::user_session::DeliveryGuard::BotInstallationScopes(vec![(
                            server_id.clone(),
                            "commands".to_owned(),
                        )]),
                    ),
                );
            }
        }
        Ok(())
    }
}
