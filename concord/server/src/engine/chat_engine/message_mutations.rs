use super::{Arc, ChatEngine, ChatEvent, ConnectionId};

impl ChatEngine {
    pub async fn submit_delete_message(
        &self,
        session_id: ConnectionId,
        command: crate::engine::messaging::EntityCommand<'_>,
    ) -> Result<crate::engine::messaging::CommandReceipt, crate::engine::messaging::MessagingError>
    {
        let actor = self.actor_for_session(session_id)?;
        let mutation = self
            .messaging_service()?
            .delete_message(&actor, command)
            .await?;
        if !mutation.receipt.replayed {
            let id =
                crate::engine::ids::MessageId::from_stored(mutation.receipt.message_id.clone())
                    .map_err(|_| crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let channel = self
                .resolve_channel_name_from_id(&mutation.channel_id)
                .unwrap_or(mutation.channel_id.clone());
            self.broadcast_to_channel_guarded(
                &mutation.channel_id,
                &mutation.conversation_id,
                &ChatEvent::MessageDelete {
                    id,
                    server_id: mutation.server_id,
                    channel,
                },
                None,
            );
        }
        self.send_committed_receipt(session_id, &mutation.receipt);
        Ok(mutation.receipt)
    }
    pub async fn submit_reaction(
        &self,
        session_id: ConnectionId,
        command: crate::engine::messaging::ReactionCommand<'_>,
        add: bool,
    ) -> Result<crate::engine::messaging::CommandReceipt, crate::engine::messaging::MessagingError>
    {
        let actor = self.actor_for_session(session_id)?;
        let mutation = self
            .messaging_service()?
            .change_reaction(&actor, command, add)
            .await?;
        if !mutation.receipt.replayed {
            let message_id =
                crate::engine::ids::MessageId::from_stored(mutation.receipt.message_id.clone())
                    .map_err(|_| crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let channel = self
                .resolve_channel_name_from_id(&mutation.channel_id)
                .unwrap_or(mutation.channel_id.clone());
            let nickname = self
                .sessions
                .get(&session_id)
                .map(|session| session.nickname.clone())
                .unwrap_or_default();
            let event = if add {
                ChatEvent::ReactionAdd {
                    message_id,
                    server_id: mutation.server_id,
                    channel,
                    user_id: mutation.actor_id,
                    nickname,
                    emoji: mutation.emoji.unwrap_or_default(),
                }
            } else {
                ChatEvent::ReactionRemove {
                    message_id,
                    server_id: mutation.server_id,
                    channel,
                    user_id: mutation.actor_id,
                    nickname,
                    emoji: mutation.emoji.unwrap_or_default(),
                }
            };
            self.broadcast_to_channel_guarded(
                &mutation.channel_id,
                &mutation.conversation_id,
                &event,
                None,
            );
        }
        self.send_committed_receipt(session_id, &mutation.receipt);
        Ok(mutation.receipt)
    }
    pub async fn submit_mark_read(
        &self,
        session_id: ConnectionId,
        command: crate::engine::messaging::ReadCommand<'_>,
    ) -> Result<crate::engine::messaging::CommandReceipt, crate::engine::messaging::MessagingError>
    {
        let actor = self.actor_for_session(session_id)?;
        let receipt = self.messaging_service()?.mark_read(&actor, command).await?;
        self.send_committed_receipt(session_id, &receipt);
        Ok(receipt)
    }
    pub(super) fn actor_for_session(
        &self,
        session_id: ConnectionId,
    ) -> Result<crate::auth::authority::Actor, crate::engine::messaging::MessagingError> {
        self.authenticated_actors
            .get(&session_id)
            .map(|actor| actor.clone())
            .ok_or(crate::engine::messaging::MessagingError::Unauthenticated)
    }
    pub(super) fn messaging_service(
        &self,
    ) -> Result<&crate::engine::messaging::MessagingService, crate::engine::messaging::MessagingError>
    {
        self.messaging
            .get()
            .ok_or(crate::engine::messaging::MessagingError::DependencyUnavailable)
    }
    pub(super) fn integration_service(
        &self,
    ) -> Result<crate::engine::integrations::IntegrationService, String> {
        Ok(crate::engine::integrations::IntegrationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
            self.integration_vault
                .get()
                .ok_or("Integration credential vault unavailable")?
                .clone(),
        ))
    }
    pub fn configure_integration_vault(
        &self,
        vault: Arc<crate::secrets::SecretVault>,
    ) -> Result<(), String> {
        self.integration_vault
            .set(vault)
            .map_err(|_| "Integration credential vault already configured".into())
    }
    pub(super) fn send_committed_receipt(
        &self,
        session_id: ConnectionId,
        receipt: &crate::engine::messaging::CommandReceipt,
    ) {
        if let Some(session) = self.sessions.get(&session_id) {
            let _ = session.send(ChatEvent::CommandCommitted {
                receipt: receipt.clone(),
            });
        }
    }
}
