use super::{
    Actor, CommandReceipt, ConversationAction, EventIdentity, MAX_ATTACHMENTS, MessageTarget,
    MessagingError, MessagingService, RATE_WINDOW_MESSAGES, RATE_WINDOW_SECONDS,
    SendDirectMessageCommand, Utc, Uuid, database_generation, hash_json, insert_event,
    insert_receipt, load_receipt, map_authorization_error, mutation_receipt, operation_generation,
    set_entity_version, validate_attachments, validate_operation_ids, validate_reply, validation,
};

impl MessagingService {
    pub async fn send_direct_message(
        &self,
        actor: &Actor,
        command: SendDirectMessageCommand<'_>,
    ) -> Result<CommandReceipt, MessagingError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::MessageCommit);
        validate_operation_ids(command.request_id, command.client_message_id)?;
        if command.recipient.is_empty() || command.recipient.len() > 256 {
            return Err(MessagingError::InvalidInput("invalid recipient".into()));
        }
        if command.attachment_ids.len() > MAX_ATTACHMENTS {
            return Err(MessagingError::InvalidInput("too many attachments".into()));
        }
        if command.content.is_empty() && command.attachment_ids.is_empty() {
            return Err(MessagingError::InvalidInput(
                "message content or an attachment is required".into(),
            ));
        }
        if !command.content.is_empty() {
            validation::validate_message_with_limit(command.content, self.max_message_length)
                .map_err(MessagingError::InvalidInput)?;
        }
        let (_permit, mut transaction) = self.begin_write().await?;
        let recipient_id: String = sqlx::query_scalar(
            "SELECT a.user_id FROM user_aliases a JOIN users u ON u.id=a.user_id \
             WHERE a.alias=? COLLATE NOCASE AND u.disabled_at IS NULL LIMIT 1",
        )
        .bind(command.recipient)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        if recipient_id == actor.user_id().as_str() {
            return Err(MessagingError::InvalidInput(
                "cannot send a direct message to yourself".into(),
            ));
        }
        let (lower_user_id, upper_user_id) = if actor.user_id().as_str() < recipient_id.as_str() {
            (actor.user_id().as_str(), recipient_id.as_str())
        } else {
            (recipient_id.as_str(), actor.user_id().as_str())
        };
        let existing_conversation_id: Option<String> = sqlx::query_scalar(
            "SELECT conversation_id FROM direct_conversation_pairs \
             WHERE lower_user_id=? AND upper_user_id=?",
        )
        .bind(lower_user_id)
        .bind(upper_user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let conversation_id = if let Some(existing) = existing_conversation_id {
            existing
        } else {
            let derived: String = sqlx::query_scalar(
                "SELECT 'direct:' || hex(CAST(? AS BLOB)) || ':' || hex(CAST(? AS BLOB))",
            )
            .bind(lower_user_id)
            .bind(upper_user_id)
            .fetch_one(&mut *transaction)
            .await?;
            sqlx::query("INSERT OR IGNORE INTO conversations(id,kind) VALUES(?,'direct')")
                .bind(&derived)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "INSERT OR IGNORE INTO direct_conversation_pairs( \
                     conversation_id,lower_user_id,upper_user_id \
                 ) VALUES(?,?,?)",
            )
            .bind(&derived)
            .bind(lower_user_id)
            .bind(upper_user_id)
            .execute(&mut *transaction)
            .await?;
            let resolved: String = sqlx::query_scalar(
                "SELECT conversation_id FROM direct_conversation_pairs \
                 WHERE lower_user_id=? AND upper_user_id=?",
            )
            .bind(lower_user_id)
            .bind(upper_user_id)
            .fetch_one(&mut *transaction)
            .await?;
            if resolved != derived {
                sqlx::query(
                    "DELETE FROM conversations WHERE id=? AND NOT EXISTS ( \
                         SELECT 1 FROM direct_conversation_pairs WHERE conversation_id=? \
                     )",
                )
                .bind(&derived)
                .bind(&derived)
                .execute(&mut *transaction)
                .await?;
            }
            resolved
        };
        for participant in [lower_user_id, upper_user_id] {
            sqlx::query(
                "INSERT OR IGNORE INTO conversation_participants(conversation_id,user_id) \
                 VALUES(?,?)",
            )
            .bind(&conversation_id)
            .bind(participant)
            .execute(&mut *transaction)
            .await?;
        }
        self.authorization
            .authorize_conversation_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &conversation_id,
                ConversationAction::Send,
            )
            .await
            .map_err(map_authorization_error)?;
        let database_generation = database_generation(&mut transaction).await?;
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "send_direct",
            "conversation_id": conversation_id,
            "recipient_id": recipient_id,
            "content": command.content,
            "content_format": command.content_format,
            "reply_to_id": command.reply_to_id,
            "attachment_ids": command.attachment_ids,
        }))?;
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
            metric.succeed();
            return Ok(receipt);
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        let recent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=? AND sender_id=? \
             AND julianday(created_at)>=julianday('now',?)",
        )
        .bind(&conversation_id)
        .bind(actor.user_id().as_str())
        .bind(format!("-{RATE_WINDOW_SECONDS} seconds"))
        .fetch_one(&mut *transaction)
        .await?;
        if recent >= RATE_WINDOW_MESSAGES {
            return Err(MessagingError::RateLimited);
        }
        validate_reply(&mut transaction, &conversation_id, command.reply_to_id).await?;
        validate_attachments(
            &mut transaction,
            actor.user_id().as_str(),
            &conversation_id,
            command.attachment_ids,
        )
        .await?;
        let sequence: i64 = sqlx::query_scalar(
            "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
             WHERE id=? AND kind='direct' RETURNING next_message_sequence",
        )
        .bind(&conversation_id)
        .fetch_one(&mut *transaction)
        .await?;
        let message_id = Uuid::new_v4().to_string();
        let persisted_at = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let sender_nick: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO messages( \
                 id,sender_id,sender_nick,target_user_id,content,created_at,reply_to_id, \
                 conversation_id,conversation_sequence,content_format,entity_version \
             ) VALUES(?,?,?,?,?,?,?,?,?,?,1)",
        )
        .bind(&message_id)
        .bind(actor.user_id().as_str())
        .bind(&sender_nick)
        .bind(&recipient_id)
        .bind(command.content)
        .bind(&persisted_at)
        .bind(command.reply_to_id)
        .bind(&conversation_id)
        .bind(sequence)
        .bind(command.content_format.as_str())
        .execute(&mut *transaction)
        .await?;
        if !command.attachment_ids.is_empty() {
            let mut builder = sqlx::QueryBuilder::new("UPDATE attachments SET message_id=");
            builder.push_bind(&message_id);
            builder
                .push(",media_state='attached',state_version=state_version+1 WHERE uploader_id=");
            builder.push_bind(actor.user_id().as_str());
            builder.push(" AND conversation_id=");
            builder.push_bind(&conversation_id);
            builder.push(
                " AND media_state='ready' AND storage_backend='local' AND storage_key IS NOT NULL \
                 AND message_id IS NULL AND id IN (",
            );
            let mut separated = builder.separated(",");
            for id in command.attachment_ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            if builder
                .build()
                .execute(&mut *transaction)
                .await?
                .rows_affected()
                != command.attachment_ids.len() as u64
            {
                return Err(MessagingError::Conflict(
                    "attachment claim changed during message acceptance".into(),
                ));
            }
        }
        set_entity_version(&mut transaction, "message", &message_id, 1).await?;
        let target = MessageTarget {
            message_id: message_id.clone(),
            conversation_id: conversation_id.clone(),
            conversation_sequence: sequence,
            server_id: String::new(),
            channel_id: String::new(),
            sender_id: actor.user_id().as_str().to_owned(),
            authorization_version: 0,
            direct: true,
            deleted: false,
        };
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: "message_created",
                entity_type: "message",
                entity_id: &message_id,
                version: 1,
            },
            actor.user_id().as_str(),
            serde_json::json!({
                "conversation_id": conversation_id,
                "message_id": message_id,
                "conversation_sequence": sequence.to_string(),
            }),
        )
        .await?;
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            &message_id,
            sequence,
            event_sequence,
            1,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            "send",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        metric.succeed();
        Ok(receipt)
    }
}
