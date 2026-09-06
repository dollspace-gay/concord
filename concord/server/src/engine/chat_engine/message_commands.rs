use super::{
    ChatEngine, ChatEvent, ConnectionId, DirectConversationInfo, Row, Utc, normalize_channel_name,
    parse_persisted_timestamp, warn,
};
use crate::engine::validation;

impl ChatEngine {
    /// Send a message to a channel or user (DM), with optional reply and attachments.
    pub async fn submit_channel_message(
        &self,
        session_id: ConnectionId,
        command: crate::engine::messaging::SendMessageCommand<'_>,
        legacy_nonce: Option<&str>,
    ) -> Result<crate::engine::messaging::CommandReceipt, crate::engine::messaging::MessagingError>
    {
        let actor = self
            .authenticated_actors
            .get(&session_id)
            .map(|entry| entry.clone())
            .ok_or(crate::engine::messaging::MessagingError::Unauthenticated)?;
        let messaging = self
            .messaging
            .get()
            .ok_or(crate::engine::messaging::MessagingError::DependencyUnavailable)?;
        let receipt = messaging
            .send_channel_message(&actor, command.clone())
            .await?;

        if let Some(session) = self.sessions.get(&session_id) {
            let id = crate::engine::ids::MessageId::from_stored(receipt.message_id.clone())
                .map_err(|_| crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let _ = session.send(ChatEvent::MessageAck {
                id,
                server_id: command.server_id.to_owned(),
                channel: normalize_channel_name(command.channel),
                conversation_id: None,
                request_id: receipt.request_id.clone(),
                client_message_id: receipt.client_message_id.clone(),
                sequence: receipt.sequence.clone(),
                persisted_at: receipt.persisted_at.clone(),
                replayed: receipt.replayed,
                nonce: legacy_nonce.map(str::to_owned),
            });
        }

        if !receipt.replayed {
            let pool = self
                .db
                .as_ref()
                .ok_or(crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            match crate::db::queries::messages::get_message_by_id(pool, &receipt.message_id).await {
                Ok(Some(row)) => {
                    if let (Some(channel_id), Some(timestamp)) = (
                        row.channel_id.as_deref(),
                        parse_persisted_timestamp(&row.created_at),
                    ) {
                        let id = crate::engine::ids::MessageId::from_stored(row.id.clone())
                            .map_err(|_| {
                                crate::engine::messaging::MessagingError::DependencyUnavailable
                            })?;
                        let event = ChatEvent::Message {
                            id,
                            server_id: row.server_id.clone(),
                            conversation_id: None,
                            from: row.sender_nick,
                            target: normalize_channel_name(command.channel),
                            content: validation::sanitize_html(&row.content),
                            timestamp,
                            avatar_url: self
                                .sessions
                                .get(&session_id)
                                .and_then(|session| session.avatar_url.clone()),
                            reply_to: None,
                            attachments: None,
                        };
                        let conversation_id: Option<String> =
                            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                                .bind(&receipt.message_id)
                                .fetch_optional(pool)
                                .await
                                .ok()
                                .flatten();
                        if let Some(conversation_id) = conversation_id {
                            self.broadcast_to_channel_guarded(
                                channel_id,
                                &conversation_id,
                                &event,
                                None,
                            );
                        }
                    } else {
                        warn!(message_id = %receipt.message_id, "committed message projection requires replay");
                    }
                }
                Ok(None) => {
                    warn!(message_id = %receipt.message_id, "committed message missing during live projection")
                }
                Err(error) => {
                    warn!(%error, message_id = %receipt.message_id, "committed message projection failed; durable replay remains pending")
                }
            }
        }
        Ok(receipt)
    }
    pub async fn list_direct_conversations(&self, session_id: ConnectionId) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        self.auth
            .get()
            .ok_or("Authentication unavailable")?
            .validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let rows = sqlx::query(
            "SELECT c.id,u.id,u.username,u.avatar_url,MAX(m.created_at), \
                    COUNT(CASE WHEN m.conversation_sequence>COALESCE(rs.conversation_sequence,0) \
                               AND m.sender_id<>? AND m.deleted_at IS NULL THEN 1 END) \
             FROM conversations c \
             JOIN conversation_participants self_cp ON self_cp.conversation_id=c.id \
                 AND self_cp.user_id=? AND self_cp.left_at IS NULL \
             JOIN conversation_participants peer_cp ON peer_cp.conversation_id=c.id \
                 AND peer_cp.user_id<>? AND peer_cp.left_at IS NULL \
             JOIN users u ON u.id=peer_cp.user_id \
             LEFT JOIN messages m ON m.conversation_id=c.id \
             LEFT JOIN read_states rs ON rs.user_id=? AND rs.channel_id=c.id \
             WHERE c.kind='direct' GROUP BY c.id,u.id,u.username,u.avatar_url \
             ORDER BY MAX(m.created_at) DESC,c.created_at DESC,c.id DESC",
        )
        .bind(actor.user_id().as_str())
        .bind(actor.user_id().as_str())
        .bind(actor.user_id().as_str())
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let conversations = rows
            .into_iter()
            .map(|row| DirectConversationInfo {
                id: row.get(0),
                peer_id: row.get(1),
                peer_username: row.get(2),
                peer_avatar_url: row.get(3),
                last_message_at: row.get(4),
                unread_count: row.get::<i64, _>(5).max(0) as u64,
            })
            .collect();
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let _ = session.send_guarded(
            ChatEvent::DirectConversationList { conversations },
            Some(crate::engine::user_session::DeliveryGuard::ActorCurrent),
        );
        Ok(())
    }
    pub async fn submit_direct_message(
        &self,
        session_id: ConnectionId,
        command: crate::engine::messaging::SendDirectMessageCommand<'_>,
        legacy_nonce: Option<&str>,
    ) -> Result<crate::engine::messaging::CommandReceipt, crate::engine::messaging::MessagingError>
    {
        let actor = self.actor_for_session(session_id)?;
        let receipt = self
            .messaging_service()?
            .send_direct_message(&actor, command.clone())
            .await?;
        let pool = self
            .db
            .as_ref()
            .ok_or(crate::engine::messaging::MessagingError::DependencyUnavailable)?;
        let conversation_id: String =
            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                .bind(&receipt.message_id)
                .fetch_one(pool)
                .await?;
        if let Some(session) = self.sessions.get(&session_id) {
            let id = crate::engine::ids::MessageId::from_stored(receipt.message_id.clone())
                .map_err(|_| crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let _ = session.send(ChatEvent::MessageAck {
                id,
                server_id: String::new(),
                channel: command.recipient.to_owned(),
                conversation_id: Some(conversation_id.clone()),
                request_id: receipt.request_id.clone(),
                client_message_id: receipt.client_message_id.clone(),
                sequence: receipt.sequence.clone(),
                persisted_at: receipt.persisted_at.clone(),
                replayed: receipt.replayed,
                nonce: legacy_nonce.map(str::to_owned),
            });
        }
        if !receipt.replayed {
            let row = sqlx::query(
                "SELECT m.sender_nick,m.target_user_id,m.content,m.created_at,u.username, \
                        m.conversation_id \
                 FROM messages m JOIN users u ON u.id=m.target_user_id WHERE m.id=?",
            )
            .bind(&receipt.message_id)
            .fetch_optional(pool)
            .await?
            .ok_or(crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let timestamp = parse_persisted_timestamp(row.get::<&str, _>(3))
                .ok_or(crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let id = crate::engine::ids::MessageId::from_stored(receipt.message_id.clone())
                .map_err(|_| crate::engine::messaging::MessagingError::DependencyUnavailable)?;
            let target_user_id: String = row.get(1);
            let event = ChatEvent::Message {
                id,
                server_id: None,
                conversation_id: Some(conversation_id.clone()),
                from: row.get(0),
                target: row.get(4),
                content: validation::sanitize_html(row.get(2)),
                timestamp,
                avatar_url: self
                    .sessions
                    .get(&session_id)
                    .and_then(|session| session.avatar_url.clone()),
                reply_to: None,
                attachments: None,
            };
            for session in self.sessions.iter() {
                if session.id != session_id
                    && (session.user_id.as_deref() == Some(target_user_id.as_str())
                        || session.user_id.as_deref() == Some(actor.user_id().as_str()))
                {
                    let _ = session.send_guarded(
                        event.clone(),
                        Some(crate::engine::user_session::DeliveryGuard::Conversations(
                            vec![conversation_id.clone()],
                        )),
                    );
                }
            }
        }
        Ok(receipt)
    }
    pub async fn submit_edit_message(
        &self,
        session_id: ConnectionId,
        command: crate::engine::messaging::EditMessageCommand<'_>,
    ) -> Result<crate::engine::messaging::CommandReceipt, crate::engine::messaging::MessagingError>
    {
        let actor = self.actor_for_session(session_id)?;
        let mutation = self
            .messaging_service()?
            .edit_message(&actor, command)
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
                &ChatEvent::MessageEdit {
                    id,
                    server_id: mutation.server_id,
                    channel,
                    content: validation::sanitize_html(mutation.content.as_deref().unwrap_or("")),
                    edited_at: Utc::now(),
                },
                None,
            );
        }
        self.send_committed_receipt(session_id, &mutation.receipt);
        Ok(mutation.receipt)
    }
}
