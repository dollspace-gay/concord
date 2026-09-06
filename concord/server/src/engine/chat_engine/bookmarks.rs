use super::{BookmarkInfo, ChatEngine, ChatEvent, ConnectionId, Row, Uuid};

impl ChatEngine {
    /// Add a bookmark on a message for the authenticated user.
    pub async fn add_bookmark(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        let msg = sqlx::query(
            "SELECT channel_id,sender_nick,content,created_at FROM messages \
             WHERE id=? AND channel_id IS NOT NULL AND deleted_at IS NULL",
        )
        .bind(message_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?
        .ok_or_else(|| "resource unavailable".to_string())?;
        let channel_id: String = msg.get(0);
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                &actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let bookmark_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO bookmarks(id,user_id,message_id,note) VALUES(?,?,?,?) \
             ON CONFLICT(user_id,message_id) DO UPDATE SET note=excluded.note",
        )
        .bind(&bookmark_id)
        .bind(actor.user_id().as_str())
        .bind(message_id)
        .bind(note)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let stored = sqlx::query(
            "SELECT id,created_at,note FROM bookmarks WHERE user_id=? AND message_id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(message_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let bookmark = BookmarkInfo {
            id: stored.get(0),
            message_id: message_id.to_string(),
            channel_id: channel_id.clone(),
            from: msg.get(1),
            content: msg.get(2),
            timestamp: msg.get(3),
            note: stored.get(2),
            created_at: stored.get(1),
        };
        let _ = session.send_guarded(
            ChatEvent::BookmarkAdd { bookmark },
            Some(crate::engine::user_session::DeliveryGuard::ChannelActions(
                vec![(
                    channel_id,
                    crate::engine::authorization::ChannelAction::ReadHistory,
                )],
            )),
        );

        Ok(())
    }
    /// Remove a bookmark for the authenticated user.
    pub async fn remove_bookmark(
        &self,
        session_id: ConnectionId,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        auth.validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        sqlx::query("DELETE FROM bookmarks WHERE user_id=? AND message_id=?")
            .bind(actor.user_id().as_str())
            .bind(message_id)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let _ = session.send_guarded(
            ChatEvent::BookmarkRemove {
                message_id: message_id.to_string(),
            },
            Some(crate::engine::user_session::DeliveryGuard::ActorCurrent),
        );

        Ok(())
    }
    /// List all bookmarks for the authenticated user. Sends BookmarkList event to the session.
    pub async fn list_bookmarks(&self, session_id: ConnectionId) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        auth.validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let rows = sqlx::query(
            "SELECT b.id,b.message_id,b.note,b.created_at,m.channel_id,m.sender_nick, \
                    m.content,m.created_at,m.deleted_at \
             FROM bookmarks b JOIN messages m ON m.id=b.message_id \
             WHERE b.user_id=? ORDER BY b.created_at DESC,b.id DESC",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let mut bookmarks = Vec::new();
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        let mut guarded_channels = Vec::new();
        for row in rows {
            let channel_id: String = row.get(4);
            if authorization
                .authorize_actor_in(
                    &mut transaction,
                    auth,
                    &actor,
                    &channel_id,
                    crate::engine::authorization::ChannelAction::ReadHistory,
                )
                .await
                .is_err()
            {
                continue;
            }
            guarded_channels.push((
                channel_id.clone(),
                crate::engine::authorization::ChannelAction::ReadHistory,
            ));
            let deleted = row.get::<Option<String>, _>(8).is_some();
            bookmarks.push(BookmarkInfo {
                id: row.get(0),
                message_id: row.get(1),
                channel_id,
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
                note: row.get(2),
                created_at: row.get(3),
            });
        }
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let _ = session.send_guarded(
            ChatEvent::BookmarkList { bookmarks },
            Some(crate::engine::user_session::DeliveryGuard::ChannelActions(
                guarded_channels,
            )),
        );

        Ok(())
    }
}
