use super::{
    ChatEngine, ChatEvent, ConnectionId, InteractionResponseData, Utc, Uuid,
    validate_rich_interaction_response,
};

impl ChatEngine {
    /// Respond to an interaction (bot -> channel).
    pub async fn respond_to_interaction(
        &self,
        session_id: ConnectionId,
        interaction_id: &str,
        content: Option<&str>,
        embeds_json: Option<&str>,
        components_json: Option<&str>,
        ephemeral: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let interaction = crate::db::queries::slash_commands::get_interaction(pool, interaction_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Interaction not found")?;

        if content.is_some_and(|value| value.len() > self.max_message_length)
            || embeds_json.is_some_and(|value| value.len() > 32 * 1024)
            || components_json.is_some_and(|value| value.len() > 32 * 1024)
        {
            return Err("Interaction response is too large".into());
        }
        let embeds = embeds_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| "Invalid interaction embeds".to_string())?;
        let components = components_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| "Invalid interaction components".to_string())?;
        if content.is_none() && embeds.is_none() && components.is_none() {
            return Err("Interaction response must contain content, embeds, or components".into());
        }
        if embeds
            .as_ref()
            .is_some_and(|values: &Vec<_>| values.len() > 10)
            || components
                .as_ref()
                .is_some_and(|values: &Vec<_>| values.len() > 5)
        {
            return Err("Interaction response contains too many embeds or components".into());
        }
        validate_rich_interaction_response(embeds.as_deref(), components.as_deref())?;

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_bot_installation_scope(auth, &actor, &interaction.server_id, "commands")
            .await
            .map_err(|error| error.to_string())?;

        let response = InteractionResponseData {
            content: content.map(String::from),
            embeds: embeds.clone(),
            components: components.clone(),
            ephemeral,
        };

        // Resolve channel name from channel_id
        let channel_name = self
            .resolve_channel_name_from_id(&interaction.channel_id)
            .unwrap_or_else(|_| interaction.channel_id.clone());

        if ephemeral {
            let response_json = serde_json::to_string(&response)
                .map_err(|_| "Invalid interaction response".to_string())?;
            let response_expires_at = (Utc::now() + chrono::Duration::minutes(15))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let mut transaction = pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(|e| format!("Failed to begin interaction response: {e}"))?;
            use crate::db::queries::slash_commands::InteractionResponseResult;
            match crate::db::queries::slash_commands::accept_interaction_response(
                &mut transaction,
                interaction_id,
                actor.user_id().as_str(),
                None,
                Some(&response_json),
                Some(&response_expires_at),
            )
            .await
            .map_err(|e| format!("Failed to accept interaction response: {e}"))?
            {
                InteractionResponseResult::Accepted => transaction
                    .commit()
                    .await
                    .map_err(|e| format!("Failed to commit interaction response: {e}"))?,
                InteractionResponseResult::AlreadyResponded => {
                    return Err("Interaction already responded".into());
                }
                InteractionResponseResult::Expired => return Err("Interaction expired".into()),
                InteractionResponseResult::WrongApplication
                | InteractionResponseResult::NotFound => {
                    return Err("Interaction not found".into());
                }
            }
            // Send only to the invoker
            for entry in self.sessions.iter() {
                let s = entry.value();
                if let Some(ref uid) = s.user_id
                    && uid == &interaction.user_id
                {
                    let _ = s.send_guarded(
                        ChatEvent::InteractionResponse {
                            interaction_id: interaction_id.to_string(),
                            server_id: interaction.server_id.clone(),
                            channel: channel_name.clone(),
                            response: response.clone(),
                        },
                        Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                            interaction.channel_id.clone(),
                        ])),
                    );
                }
            }
        } else {
            let content = content.unwrap_or_default();
            let rich_embeds_json = embeds
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| "Invalid interaction embeds".to_string())?;
            let canonical_components_json = components
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| "Invalid interaction components".to_string())?;
            let request_id = Uuid::new_v4().to_string();
            let client_message_id = format!("interaction:{interaction_id}:response:1");
            let empty_attachments: Vec<String> = Vec::new();
            let receipt = self
                .messaging_service()
                .map_err(|error| error.to_string())?
                .respond_to_interaction_public(
                    &actor,
                    interaction_id,
                    crate::engine::messaging::SendMessageCommand {
                        request_id: &request_id,
                        client_message_id: &client_message_id,
                        operation_generation: None,
                        conversation_id: None,
                        server_id: &interaction.server_id,
                        channel: &channel_name,
                        content,
                        content_format: crate::engine::messaging::ContentFormat::Markdown,
                        reply_to_id: None,
                        attachment_ids: &empty_attachments,
                        mentions: &[],
                    },
                    rich_embeds_json.as_deref(),
                    canonical_components_json.as_deref(),
                )
                .await
                .map_err(|error| error.to_string())?;
            self.send_committed_receipt(session_id, &receipt);
        }

        Ok(())
    }
}
