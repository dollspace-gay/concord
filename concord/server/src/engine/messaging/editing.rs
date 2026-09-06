use super::{
    Actor, ChannelAction, EditMessageCommand, EventIdentity, MAX_MENTIONS, MessageMutation,
    MessagingError, MessagingService, Utc, database_generation, enforce_automod, hash_json,
    insert_event, insert_mentions, insert_receipt, load_receipt, map_authorization_error,
    mutation_receipt, operation_generation, propagate_announcement_edit, set_entity_version,
    validate_mentions, validate_operation_ids, validation,
};

impl MessagingService {
    pub async fn edit_message(
        &self,
        actor: &Actor,
        command: EditMessageCommand<'_>,
    ) -> Result<MessageMutation, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        validation::validate_message_with_limit(command.content, self.max_message_length)
            .map_err(MessagingError::InvalidInput)?;
        if command.mentions.len() > MAX_MENTIONS {
            return Err(MessagingError::InvalidInput("too many mentions".into()));
        }
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "edit",
            "message_id": command.message_id,
            "content": command.content,
            "content_format": command.content_format,
            "mentions": command.mentions,
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
                content: Some(command.content.to_owned()),
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
        match enforce_automod(
            &mut transaction,
            &target.server_id,
            actor.user_id().as_str(),
            command.client_message_id,
            command.content,
        )
        .await
        {
            Ok(()) => {}
            Err(error @ MessagingError::AutoModRejected(_)) => {
                transaction.commit().await?;
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        validate_mentions(
            &mut transaction,
            &target.server_id,
            command.content,
            command.mentions,
        )
        .await?;
        let version: i64 = sqlx::query_scalar(
            "UPDATE messages SET content=?,content_format=?,edited_at=datetime('now'), \
             entity_version=entity_version+1 WHERE id=? AND deleted_at IS NULL \
             RETURNING entity_version",
        )
        .bind(command.content)
        .bind(command.content_format.as_str())
        .bind(command.message_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        sqlx::query("DELETE FROM message_mentions WHERE message_id=?")
            .bind(command.message_id)
            .execute(&mut *transaction)
            .await?;
        insert_mentions(&mut transaction, command.message_id, command.mentions).await?;
        set_entity_version(&mut transaction, "message", command.message_id, version).await?;
        crate::db::queries::atproto::schedule_source_mutation(
            &mut transaction,
            command.message_id,
            version,
            false,
        )
        .await?;
        let persisted_at = Utc::now().to_rfc3339();
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: "message_edited",
                entity_type: "message",
                entity_id: command.message_id,
                version,
            },
            actor.user_id().as_str(),
            serde_json::json!({"message_id": command.message_id}),
        )
        .await?;
        propagate_announcement_edit(
            &mut transaction,
            &database_generation,
            command.message_id,
            command.content,
            command.content_format.as_str(),
            version,
            actor.user_id().as_str(),
        )
        .await?;
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
            "edit",
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
            content: Some(command.content.to_owned()),
            emoji: None,
            actor_id: actor.user_id().as_str().to_owned(),
        })
    }
}
