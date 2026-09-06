use super::{
    Actor, EventIdentity, MessageMutation, MessagingError, MessagingService, ReactionCommand, Utc,
    advance_entity_version, database_generation, hash_json, insert_event, insert_receipt,
    load_receipt, mutation_receipt, operation_generation, reaction_entity_id,
    validate_operation_ids,
};

impl MessagingService {
    pub async fn change_reaction(
        &self,
        actor: &Actor,
        command: ReactionCommand<'_>,
        add: bool,
    ) -> Result<MessageMutation, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        if command.emoji.is_empty() || command.emoji.chars().count() > 32 {
            return Err(MessagingError::InvalidInput("invalid reaction".into()));
        }
        let operation = if add {
            "reaction_add"
        } else {
            "reaction_remove"
        };
        let fingerprint = hash_json(&serde_json::json!({
            "operation": operation,
            "message_id": command.message_id,
            "emoji": command.emoji,
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
                emoji: Some(command.emoji.to_owned()),
                actor_id: actor.user_id().as_str().to_owned(),
            });
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        if add {
            sqlx::query("INSERT OR IGNORE INTO reactions(message_id,user_id,emoji) VALUES(?,?,?)")
                .bind(command.message_id)
                .bind(actor.user_id().as_str())
                .bind(command.emoji)
                .execute(&mut *transaction)
                .await?;
        } else {
            sqlx::query("DELETE FROM reactions WHERE message_id=? AND user_id=? AND emoji=?")
                .bind(command.message_id)
                .bind(actor.user_id().as_str())
                .bind(command.emoji)
                .execute(&mut *transaction)
                .await?;
        }
        let reaction_entity =
            reaction_entity_id(command.message_id, actor.user_id().as_str(), command.emoji);
        let version =
            advance_entity_version(&mut transaction, "reaction", &reaction_entity).await?;
        let persisted_at = Utc::now().to_rfc3339();
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: if add {
                    "reaction_added"
                } else {
                    "reaction_removed"
                },
                entity_type: "reaction",
                entity_id: &reaction_entity,
                version,
            },
            actor.user_id().as_str(),
            serde_json::json!({
                "message_id": command.message_id,
                "user_id": actor.user_id().as_str(),
                "emoji": command.emoji,
                "present": add,
            }),
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
            operation,
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
            emoji: Some(command.emoji.to_owned()),
            actor_id: actor.user_id().as_str().to_owned(),
        })
    }
}
