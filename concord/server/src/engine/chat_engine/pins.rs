use super::{
    ChatEngine, ChatEvent, ConnectionId, PinnedMessageInfo, Row, Uuid, normalize_channel_name,
};

impl ChatEngine {
    /// Pin a message in a channel. Requires MANAGE_MESSAGES permission.
    pub async fn pin_message(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ManageMessages,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let msg = sqlx::query(
            "SELECT sender_nick,content,created_at FROM messages \
             WHERE id=? AND channel_id=? AND server_id=? AND deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(&channel_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?
        .ok_or_else(|| "resource unavailable".to_string())?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM pinned_messages WHERE channel_id=? AND message_id=?",
        )
        .bind(&channel_id)
        .bind(message_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        if existing.is_some() {
            transaction
                .commit()
                .await
                .map_err(|_| "resource unavailable".to_string())?;
            return Ok(());
        }
        let pin_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pinned_messages WHERE channel_id=?")
                .bind(&channel_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        if pin_count >= 50 {
            return Err("Channel has reached the maximum of 50 pinned messages".into());
        }
        let pin_id = Uuid::new_v4().to_string();
        let pinned_at: String = sqlx::query_scalar(
            "INSERT INTO pinned_messages(id,channel_id,message_id,pinned_by) VALUES(?,?,?,?) \
             RETURNING pinned_at",
        )
        .bind(&pin_id)
        .bind(&channel_id)
        .bind(message_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let pin = PinnedMessageInfo {
            id: pin_id,
            message_id: message_id.to_string(),
            channel_id: channel_id.clone(),
            pinned_by: actor.user_id().as_str().to_owned(),
            pinned_at,
            from: msg.get(0),
            content: msg.get(1),
            timestamp: msg.get(2),
        };

        let event = ChatEvent::MessagePin {
            server_id: server_id.to_string(),
            channel: channel_name,
            pin,
        };
        if let Some(channel) = self.channels.get(&channel_id) {
            for member_id in &channel.members {
                if let Some(session) = self.sessions.get(member_id) {
                    let _ = session.send_guarded(
                        event.clone(),
                        Some(crate::engine::user_session::DeliveryGuard::ChannelActions(
                            vec![(
                                channel_id.clone(),
                                crate::engine::authorization::ChannelAction::ReadHistory,
                            )],
                        )),
                    );
                }
            }
        }

        Ok(())
    }
    /// Unpin a message from a channel. Requires MANAGE_MESSAGES permission.
    pub async fn unpin_message(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ManageMessages,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let removed = sqlx::query(
            "DELETE FROM pinned_messages WHERE channel_id=? AND message_id=? \
             AND EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)",
        )
        .bind(&channel_id)
        .bind(message_id)
        .bind(&channel_id)
        .bind(server_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?
        .rows_affected();
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        if removed == 0 {
            return Ok(());
        }

        let event = ChatEvent::MessageUnpin {
            server_id: server_id.to_string(),
            channel: channel_name,
            message_id: message_id.to_string(),
        };
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }
    /// Get all pinned messages in a channel. Sends PinnedMessages event to the requesting session.
    pub async fn get_pinned_messages(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        let pin_rows = sqlx::query(
            "SELECT p.id,p.message_id,p.channel_id,p.pinned_by,p.pinned_at, \
                    m.sender_nick,m.content,m.created_at,m.deleted_at \
             FROM pinned_messages p JOIN messages m ON m.id=p.message_id \
             WHERE p.channel_id=? ORDER BY p.pinned_at DESC,p.id DESC",
        )
        .bind(&channel_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;

        let mut pins = Vec::new();
        for row in pin_rows {
            let deleted = row.get::<Option<String>, _>(8).is_some();
            pins.push(PinnedMessageInfo {
                id: row.get(0),
                message_id: row.get(1),
                channel_id: row.get(2),
                pinned_by: row.get(3),
                pinned_at: row.get(4),
                from: if deleted {
                    "unknown".to_owned()
                } else {
                    row.get(5)
                },
                content: if deleted {
                    "[deleted]".to_owned()
                } else {
                    row.get(6)
                },
                timestamp: if deleted { String::new() } else { row.get(7) },
            });
        }
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        let _ = session.send_guarded(
            ChatEvent::PinnedMessages {
                server_id: server_id.to_string(),
                channel: channel_name,
                pins,
            },
            Some(crate::engine::user_session::DeliveryGuard::ChannelActions(
                vec![(
                    channel_id,
                    crate::engine::authorization::ChannelAction::ReadHistory,
                )],
            )),
        );

        Ok(())
    }
}
