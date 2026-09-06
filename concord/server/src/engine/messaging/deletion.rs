use super::{
    Actor, ChannelAction, EntityCommand, MessageMutation, MessagingError, MessagingService, Utc,
    database_generation, hash_json, insert_receipt, load_receipt, map_authorization_error,
    mutation_receipt, operation_generation, tombstone_message_in, validate_operation_ids,
};

impl MessagingService {
    pub async fn delete_message(
        &self,
        actor: &Actor,
        command: EntityCommand<'_>,
    ) -> Result<MessageMutation, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "delete",
            "message_id": command.message_id,
        }))?;
        let (_permit, mut transaction) = self.begin_write().await?;
        let target = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, true)
            .await?;
        let database_generation = database_generation(&mut transaction).await?;
        if let Some(receipt) = load_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            command.client_message_id,
            &fingerprint,
            command.request_id,
        )
        .await?
        {
            transaction.commit().await?;
            return Ok(MessageMutation {
                receipt,
                conversation_id: target.conversation_id,
                channel_id: target.channel_id,
                server_id: target.server_id,
                content: None,
                emoji: None,
                actor_id: actor.user_id().as_str().to_owned(),
            });
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        if target.sender_id != actor.user_id().as_str() {
            if target.direct {
                return Err(MessagingError::Unavailable);
            }
            self.authorization
                .authorize_actor_in(
                    &mut transaction,
                    &self.auth,
                    actor,
                    &target.channel_id,
                    ChannelAction::ManageMessages,
                )
                .await
                .map_err(map_authorization_error)?;
        }
        let (version, event_sequence) = tombstone_message_in(
            &mut transaction,
            &database_generation,
            &target,
            actor.user_id().as_str(),
        )
        .await?;
        let persisted_at = Utc::now().to_rfc3339();
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            command.message_id,
            target.conversation_sequence,
            event_sequence,
            version,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            "delete",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        Ok(MessageMutation {
            receipt,
            conversation_id: target.conversation_id,
            channel_id: target.channel_id,
            server_id: target.server_id,
            content: None,
            emoji: None,
            actor_id: actor.user_id().as_str().to_owned(),
        })
    }
}
