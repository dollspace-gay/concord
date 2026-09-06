use super::{
    Actor, CommandReceipt, EventIdentity, MessagingError, MessagingService, ReadCommand, Utc,
    advance_entity_version, database_generation, hash_json, insert_event, insert_receipt,
    load_receipt, mutation_receipt, operation_generation, read_entity_id, validate_operation_ids,
};

impl MessagingService {
    pub async fn mark_read(
        &self,
        actor: &Actor,
        command: ReadCommand<'_>,
    ) -> Result<CommandReceipt, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "read",
            "conversation_id": command.conversation_id,
            "message_id": command.message_id,
        }))?;
        let (_permit, mut transaction) = self.begin_write().await?;
        let target = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, true)
            .await?;
        if target.conversation_id != command.conversation_id {
            return Err(MessagingError::Unavailable);
        }
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
            return Ok(receipt);
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        let read_key = if target.direct {
            &target.conversation_id
        } else {
            &target.channel_id
        };
        let current_read_sequence: Option<i64> = sqlx::query_scalar(
            "SELECT conversation_sequence FROM read_states WHERE user_id=? AND channel_id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(read_key)
        .fetch_optional(&mut *transaction)
        .await?;
        if current_read_sequence.is_some_and(|sequence| sequence >= target.conversation_sequence) {
            let read_entity = read_entity_id(actor.user_id().as_str(), command.conversation_id);
            let (version, event_sequence): (i64, i64) = sqlx::query_as(
                "SELECT \
                    COALESCE((SELECT version FROM entity_versions \
                              WHERE entity_type='read_state' AND entity_id=?),1), \
                    COALESCE((SELECT MAX(event_sequence) FROM event_log \
                              WHERE entity_type='read_state' AND entity_id=?),0)",
            )
            .bind(&read_entity)
            .bind(&read_entity)
            .fetch_one(&mut *transaction)
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
                "read",
                &fingerprint,
                &target,
                &receipt,
            )
            .await?;
            transaction.commit().await?;
            return Ok(receipt);
        }
        sqlx::query(
            "INSERT INTO read_states( \
                 user_id,channel_id,last_read_message_id,last_read_at,conversation_sequence \
             ) VALUES(?,?,?,datetime('now'),?) \
             ON CONFLICT(user_id,channel_id) DO UPDATE SET \
                 last_read_message_id=CASE \
                     WHEN excluded.conversation_sequence > read_states.conversation_sequence \
                     THEN excluded.last_read_message_id ELSE read_states.last_read_message_id END, \
                 conversation_sequence=MAX(read_states.conversation_sequence,excluded.conversation_sequence), \
                 last_read_at=CASE \
                     WHEN excluded.conversation_sequence > read_states.conversation_sequence \
                     THEN excluded.last_read_at ELSE read_states.last_read_at END",
        )
        .bind(actor.user_id().as_str())
        .bind(read_key)
        .bind(command.message_id)
        .bind(target.conversation_sequence)
        .execute(&mut *transaction)
        .await?;
        let read_entity = read_entity_id(actor.user_id().as_str(), command.conversation_id);
        let version = advance_entity_version(&mut transaction, "read_state", &read_entity).await?;
        let persisted_at = Utc::now().to_rfc3339();
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: "read_advanced",
                entity_type: "read_state",
                entity_id: &read_entity,
                version,
            },
            actor.user_id().as_str(),
            serde_json::json!({
                "user_id": actor.user_id().as_str(),
                "conversation_id": command.conversation_id,
                "message_id": command.message_id,
                "conversation_sequence": target.conversation_sequence.to_string(),
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
            "read",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        Ok(receipt)
    }
}
