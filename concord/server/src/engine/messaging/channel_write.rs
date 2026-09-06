use super::{
    Actor, ChannelAction, CommandReceipt, EventIdentity, MessageTarget, MessagingError,
    MessagingService, Row, SendMessageCommand, SqliteConnection, Utc, Uuid, database_generation,
    enforce_automod, enforce_rate_and_slow_mode, enforce_timeout, enqueue_outgoing_webhooks,
    hash_json, map_authorization_error, normalize_channel_name, operation_generation,
    validate_attachments, validate_mentions, validate_reply,
};

impl MessagingService {
    pub(super) async fn send_channel_message_in(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
        command: &SendMessageCommand<'_>,
        content: &str,
    ) -> Result<CommandReceipt, MessagingError> {
        let channel = sqlx::query(
            "SELECT c.id,c.server_id,c.archived,c.slowmode_seconds,c.authorization_version,cv.id \
             FROM channels c JOIN conversations cv ON cv.channel_id=c.id AND cv.kind='channel' \
             WHERE ((? IS NOT NULL AND cv.id=?) \
                 OR (? IS NULL AND c.server_id=? AND c.name=?))",
        )
        .bind(command.conversation_id)
        .bind(command.conversation_id)
        .bind(command.conversation_id)
        .bind(command.server_id)
        .bind(normalize_channel_name(command.channel))
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        let channel_id: String = channel.get(0);
        let canonical_server_id: String = channel.get(1);
        let conversation_id: String = channel.get(5);
        if !command.server_id.is_empty() && command.server_id != canonical_server_id {
            return Err(MessagingError::Unavailable);
        }
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "send",
            "conversation_id": conversation_id,
            "content": content,
            "content_format": command.content_format,
            "reply_to_id": command.reply_to_id,
            "attachment_ids": command.attachment_ids,
            "mentions": command.mentions,
        }))?;

        self.authorization
            .authorize_actor_in(
                connection,
                &self.auth,
                actor,
                &channel_id,
                ChannelAction::Send,
            )
            .await
            .map_err(map_authorization_error)?;

        let database_generation = database_generation(connection).await?;
        if let Some(existing) = sqlx::query(
            "SELECT payload_fingerprint,response_json FROM command_receipts \
             WHERE principal_id=? AND client_message_id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(command.client_message_id)
        .fetch_optional(&mut *connection)
        .await?
        {
            if existing.get::<String, _>(0) != fingerprint {
                return Err(MessagingError::IdempotencyConflict);
            }
            let mut receipt: CommandReceipt = serde_json::from_str(existing.get::<&str, _>(1))
                .map_err(|_| MessagingError::DependencyUnavailable)?;
            receipt.request_id = command.request_id.to_owned();
            receipt.replayed = true;
            receipt.event_sequence_internal = sqlx::query_scalar::<_, i64>(
                "SELECT event_sequence FROM command_receipts \
                 WHERE principal_id=? AND client_message_id=?",
            )
            .bind(actor.user_id().as_str())
            .bind(command.client_message_id)
            .fetch_one(&mut *connection)
            .await? as u64;
            return Ok(receipt);
        }
        let operation_generation =
            operation_generation(connection, command.operation_generation).await?;

        if channel.get::<i64, _>(2) != 0 {
            return Err(MessagingError::Conflict(
                "archived channel does not accept messages".into(),
            ));
        }
        enforce_timeout(connection, &canonical_server_id, actor.user_id().as_str()).await?;
        enforce_rate_and_slow_mode(
            connection,
            &channel_id,
            actor.user_id().as_str(),
            channel.get(3),
        )
        .await?;
        enforce_automod(
            connection,
            &canonical_server_id,
            actor.user_id().as_str(),
            command.client_message_id,
            content,
        )
        .await?;
        validate_reply(connection, &conversation_id, command.reply_to_id).await?;
        validate_attachments(
            connection,
            actor.user_id().as_str(),
            &conversation_id,
            command.attachment_ids,
        )
        .await?;
        validate_mentions(connection, &canonical_server_id, content, command.mentions).await?;

        let sequence: i64 = sqlx::query_scalar(
            "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
             WHERE id=? RETURNING next_message_sequence",
        )
        .bind(&conversation_id)
        .fetch_one(&mut *connection)
        .await?;
        let message_id = Uuid::new_v4().to_string();
        let persisted_at = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let sender_nick: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&mut *connection)
            .await?;
        sqlx::query(
            "INSERT INTO messages( \
                 id,server_id,channel_id,sender_id,sender_nick,content,created_at,reply_to_id, \
                 conversation_id,conversation_sequence,content_format,entity_version \
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,1)",
        )
        .bind(&message_id)
        .bind(&canonical_server_id)
        .bind(&channel_id)
        .bind(actor.user_id().as_str())
        .bind(&sender_nick)
        .bind(content)
        .bind(&persisted_at)
        .bind(command.reply_to_id)
        .bind(&conversation_id)
        .bind(sequence)
        .bind(command.content_format.as_str())
        .execute(&mut *connection)
        .await?;
        crate::db::queries::threads::record_thread_activity(connection, &channel_id, &persisted_at)
            .await?;

        if !command.attachment_ids.is_empty() {
            let mut builder = sqlx::QueryBuilder::new("UPDATE attachments SET message_id=");
            builder.push_bind(&message_id);
            builder.push(",media_state='attached',state_version=state_version+1");
            builder.push(" WHERE uploader_id=");
            builder.push_bind(actor.user_id().as_str());
            builder.push(" AND conversation_id=");
            builder.push_bind(&conversation_id);
            builder.push(
                " AND media_state='ready' AND storage_backend='local' \
                           AND storage_key IS NOT NULL AND message_id IS NULL AND id IN (",
            );
            let mut separated = builder.separated(",");
            for id in command.attachment_ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            let changed = builder.build().execute(&mut *connection).await?;
            if changed.rows_affected() != command.attachment_ids.len() as u64 {
                return Err(MessagingError::Conflict(
                    "attachment claim changed during message acceptance".into(),
                ));
            }
        }
        for (ordinal, mention) in command.mentions.iter().enumerate() {
            sqlx::query(
                "INSERT INTO message_mentions( \
                    message_id,ordinal,mention_kind,target_id,start_byte,end_byte \
                 ) VALUES(?,?,?,?,?,?)",
            )
            .bind(&message_id)
            .bind(ordinal as i64)
            .bind(mention.kind.as_str())
            .bind(&mention.target_id)
            .bind(mention.start_byte as i64)
            .bind(mention.end_byte as i64)
            .execute(&mut *connection)
            .await?;
        }
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version) VALUES('message',?,1)",
        )
        .bind(&message_id)
        .execute(&mut *connection)
        .await?;

        let descriptor = serde_json::json!({
            "conversation_id": conversation_id,
            "message_id": message_id,
            "conversation_sequence": sequence.to_string(),
        });
        let event_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'message_created','message',?,1,?,?,?) RETURNING event_sequence",
        )
        .bind(&database_generation)
        .bind(&conversation_id)
        .bind(&message_id)
        .bind(channel.get::<i64, _>(4))
        .bind(actor.user_id().as_str())
        .bind(descriptor.to_string())
        .fetch_one(&mut *connection)
        .await?;
        sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
            .bind(event_sequence)
            .execute(&mut *connection)
            .await?;
        enqueue_outgoing_webhooks(
            connection,
            event_sequence,
            &MessageTarget {
                message_id: message_id.clone(),
                conversation_id: conversation_id.clone(),
                conversation_sequence: sequence,
                server_id: canonical_server_id.clone(),
                channel_id: channel_id.clone(),
                sender_id: actor.user_id().as_str().to_owned(),
                authorization_version: channel.get(4),
                direct: false,
                deleted: false,
            },
            &EventIdentity {
                kind: "message_created",
                entity_type: "message",
                entity_id: &message_id,
                version: 1,
            },
            actor.user_id().as_str(),
            &descriptor,
        )
        .await?;

        let receipt = CommandReceipt {
            request_id: command.request_id.to_owned(),
            client_message_id: command.client_message_id.to_owned(),
            message_id: message_id.clone(),
            sequence: sequence.to_string(),
            entity_version: 1,
            persisted_at: persisted_at.clone(),
            replayed: false,
            event_sequence_internal: event_sequence as u64,
        };
        sqlx::query(
            "INSERT INTO command_receipts( \
                principal_id,operation_generation,client_message_id,request_id,operation_kind, \
                payload_fingerprint,conversation_id,canonical_message_id,conversation_sequence, \
                event_sequence,entity_version,persisted_at,response_json \
             ) VALUES(?,?,?,?,'send',?,?,?,?,?,1,?,?)",
        )
        .bind(actor.user_id().as_str())
        .bind(&operation_generation)
        .bind(command.client_message_id)
        .bind(command.request_id)
        .bind(&fingerprint)
        .bind(&conversation_id)
        .bind(&message_id)
        .bind(sequence)
        .bind(event_sequence)
        .bind(&persisted_at)
        .bind(serde_json::to_string(&receipt).expect("receipt serialization is infallible"))
        .execute(&mut *connection)
        .await?;
        Ok(receipt)
    }
}
