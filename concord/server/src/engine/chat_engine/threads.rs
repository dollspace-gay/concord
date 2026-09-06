use super::{
    ChannelState, ChatEngine, ChatEvent, ConnectionId, ThreadInfo, Utc, Uuid,
    normalize_channel_name,
};

impl ChatEngine {
    /// Create a thread from a message in a channel.
    pub async fn create_thread(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        parent_channel_name: &str,
        name: &str,
        message_id: &str,
        is_private: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;

        let parent_channel_name = normalize_channel_name(parent_channel_name);
        let parent_channel_id = self.resolve_channel_id(server_id, &parent_channel_name)?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        crate::engine::authorization::AuthorizationService::new(
            self.db.clone().ok_or("No database configured")?,
        )
        .authorize_actor_in(
            &mut transaction,
            self.auth.get().ok_or("Authentication unavailable")?,
            &actor,
            &parent_channel_id,
            crate::engine::authorization::ChannelAction::Send,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;

        // Validate thread name
        if name.is_empty() || name.len() > 100 {
            return Err("Thread name must be between 1 and 100 characters".into());
        }

        let channel_type = if is_private {
            "private_thread"
        } else {
            "public_thread"
        };

        let thread_id = Uuid::new_v4().to_string();
        let thread_name = normalize_channel_name(name);

        // Check name uniqueness within server
        if self
            .channel_name_index
            .contains_key(&(server_id.to_string(), thread_name.clone()))
        {
            return Err(format!(
                "A channel or thread named {thread_name} already exists"
            ));
        }

        crate::db::queries::threads::create_thread_in(
            &mut transaction,
            &crate::db::queries::threads::CreateThreadParams {
                channel_id: &thread_id,
                server_id,
                name: &thread_name,
                channel_type,
                parent_message_id: message_id,
                parent_channel_id: &parent_channel_id,
                creator_user_id: actor.user_id().as_str(),
                auto_archive_minutes: 1440,
            },
        )
        .await
        .map_err(|e| format!("Failed to create thread: {e}"))?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;

        // Add to in-memory state
        let mut ch = ChannelState::new(
            thread_id.clone(),
            server_id.to_string(),
            thread_name.clone(),
        );
        ch.channel_type = channel_type.to_string();
        ch.thread_parent_message_id = Some(message_id.to_string());
        ch.thread_creator_user_id = Some(actor.user_id().as_str().to_string());
        ch.auto_archive_minutes = 1440;
        ch.is_private = is_private;

        self.channel_name_index.insert(
            (server_id.to_string(), thread_name.clone()),
            thread_id.clone(),
        );
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.insert(thread_id.clone());
        }
        self.channels.insert(thread_id.clone(), ch);

        let thread_info = ThreadInfo {
            id: thread_id.clone(),
            name: thread_name,
            channel_type: channel_type.to_string(),
            parent_message_id: Some(message_id.to_string()),
            creator_user_id: Some(actor.user_id().as_str().to_string()),
            archived: false,
            state_version: 1,
            tags_version: 1,
            tag_ids: Vec::new(),
            auto_archive_minutes: 1440,
            message_count: 0,
            created_at: Utc::now().to_rfc3339(),
        };

        let event = ChatEvent::ThreadCreate {
            server_id: server_id.to_string(),
            parent_channel: parent_channel_name,
            thread: thread_info,
        };
        if is_private {
            if let Some(connections) = self.user_connections.get(actor.user_id().as_str()) {
                for session_id in connections.iter() {
                    if let Some(session) = self.sessions.get(session_id) {
                        let _ = session.send_guarded(
                            event.clone(),
                            Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                                thread_id.clone(),
                            ])),
                        );
                    }
                }
            }
        } else {
            self.broadcast_to_channel(&parent_channel_id, &event, None);
        }

        Ok(())
    }
    /// Archive a thread. Requires MANAGE_CHANNELS permission.
    pub async fn archive_thread(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        self.set_thread_archived(session_id, server_id, thread_id, true)
            .await
    }
    pub async fn unarchive_thread(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        self.set_thread_archived(session_id, server_id, thread_id, false)
            .await
    }
    pub(super) async fn set_thread_archived(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
        archived: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        crate::engine::authorization::AuthorizationService::new(
            self.db.clone().ok_or("No database configured")?,
        )
        .authorize_actor_in(
            &mut transaction,
            self.auth.get().ok_or("Authentication unavailable")?,
            &actor,
            thread_id,
            crate::engine::authorization::ChannelAction::Manage,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let actual_server: Option<String> =
            sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                .bind(thread_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
        if actual_server.as_deref() != Some(server_id) {
            return Err("resource unavailable".into());
        }
        let version = crate::db::queries::threads::set_thread_archived_in(
            &mut transaction,
            thread_id,
            archived,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        Self::insert_thread_state_event_in(
            &mut transaction,
            thread_id,
            version,
            archived,
            archived.then_some("manual"),
            actor.user_id().as_str(),
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        self.project_thread_state(thread_id).await
    }
    /// List threads for a channel. Sends ThreadList event to the requesting session.
    pub async fn list_threads(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let rows = crate::engine::authorization::AuthorizationService::new(pool.clone())
            .visible_channels(user_id, server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?
            .into_iter()
            .filter(|row| row.parent_channel_id.as_deref() == Some(channel_id.as_str()))
            .collect::<Vec<_>>();

        let mut guarded_channels = vec![channel_id];
        guarded_channels.extend(rows.iter().map(|row| row.id.clone()));
        let mut threads = Vec::with_capacity(rows.len());
        for row in rows {
            let tag_ids = sqlx::query_scalar(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(&row.id)
            .fetch_all(pool)
            .await
            .map_err(|error| format!("Failed to list thread tags: {error}"))?;
            threads.push(ThreadInfo {
                id: row.id,
                name: row.name,
                channel_type: row.channel_type,
                parent_message_id: row.thread_parent_message_id,
                creator_user_id: row.thread_creator_user_id,
                archived: row.archived != 0,
                state_version: row.thread_state_version,
                tags_version: row.thread_tags_version,
                tag_ids,
                auto_archive_minutes: row.thread_auto_archive_minutes,
                message_count: 0, // would need a count query; returning 0 for now
                created_at: row.created_at,
            });
        }

        let _ = session.send_guarded(
            ChatEvent::ThreadList {
                server_id: server_id.to_string(),
                channel: channel_name,
                threads,
            },
            Some(crate::engine::user_session::DeliveryGuard::Channels(
                guarded_channels,
            )),
        );

        Ok(())
    }
}
