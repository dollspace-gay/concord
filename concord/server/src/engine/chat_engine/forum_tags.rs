use super::{ChatEngine, ChatEvent, ConnectionId, normalize_channel_name};

impl ChatEngine {
    pub async fn create_forum_tag(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        name: &str,
        emoji: Option<&str>,
        moderated: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = crate::engine::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let tag = service
            .create_tag(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                crate::engine::forum::CreateForumTag {
                    server_id,
                    channel_id: &channel_id,
                    name,
                    emoji,
                    moderated,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;
        let event = ChatEvent::ForumTagUpdate {
            server_id: server_id.to_string(),
            channel: channel_name,
            tag,
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                event.clone(),
                Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }
        self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn update_forum_tag(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        tag_id: &str,
        name: &str,
        emoji: Option<&str>,
        moderated: bool,
        position: i32,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = crate::engine::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let tag = service
            .update_tag(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                crate::engine::forum::UpdateForumTag {
                    server_id,
                    channel_id: &channel_id,
                    tag_id,
                    name,
                    emoji,
                    moderated,
                    position,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;
        let event = ChatEvent::ForumTagUpdate {
            server_id: server_id.to_string(),
            channel: channel_name,
            tag,
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                event.clone(),
                Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }
        self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        Ok(())
    }
    pub async fn delete_forum_tag(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = crate::engine::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let mutations = service
            .delete_tag(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                &channel_id,
                tag_id,
            )
            .await
            .map_err(|error| error.wire_message())?;
        for mutation in mutations {
            if let Some(mut thread) = self.channels.get_mut(&mutation.thread_id)
                && mutation.version >= thread.thread_tags_version
            {
                thread.thread_tags_version = mutation.version;
                thread.thread_tag_ids = mutation.tag_ids.clone();
            }
            let event = ChatEvent::ThreadTagUpdate {
                server_id: server_id.to_string(),
                thread_id: mutation.thread_id.clone(),
                version: mutation.version,
                tag_ids: mutation.tag_ids,
            };
            if let Some(session) = self.get_session(session_id) {
                let _ = session.send_guarded(
                    event.clone(),
                    Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                        mutation.thread_id.clone(),
                    ])),
                );
            }
            self.broadcast_to_channel(&mutation.thread_id, &event, Some(session_id));
        }
        let event = ChatEvent::ForumTagDelete {
            server_id: server_id.to_string(),
            channel: channel_name,
            tag_id: tag_id.to_string(),
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                event.clone(),
                Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }
        self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        Ok(())
    }
    pub async fn list_forum_tags(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let session = self.get_session(session_id).ok_or("resource unavailable")?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = crate::engine::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let tags = service
            .list_tags(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                &channel_id,
            )
            .await
            .map_err(|error| error.wire_message())?;
        let _ = session.send_guarded(
            ChatEvent::ForumTagList {
                server_id: server_id.to_string(),
                channel: channel_name,
                tags,
            },
            Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                channel_id,
            ])),
        );
        Ok(())
    }
    pub async fn set_thread_tags(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
        tag_ids: Vec<String>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let service = crate::engine::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let mutation = service
            .set_thread_tags(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                thread_id,
                tag_ids,
            )
            .await
            .map_err(|error| error.wire_message())?;
        if let Some(mut channel) = self.channels.get_mut(thread_id)
            && mutation.version >= channel.thread_tags_version
        {
            channel.thread_tags_version = mutation.version;
            channel.thread_tag_ids = mutation.tag_ids.clone();
        }
        self.broadcast_to_channel(
            thread_id,
            &ChatEvent::ThreadTagUpdate {
                server_id: server_id.to_string(),
                thread_id: thread_id.to_string(),
                version: mutation.version,
                tag_ids: mutation.tag_ids,
            },
            None,
        );
        Ok(())
    }
    pub async fn get_thread_tags(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let session = self.get_session(session_id).ok_or("resource unavailable")?;
        let service = crate::engine::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let (version, tag_ids) = service
            .get_thread_tags(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                thread_id,
            )
            .await
            .map_err(|error| error.wire_message())?;
        let _ = session.send_guarded(
            ChatEvent::ThreadTagUpdate {
                server_id: server_id.to_string(),
                thread_id: thread_id.to_string(),
                version,
                tag_ids,
            },
            Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                thread_id.to_string(),
            ])),
        );
        Ok(())
    }
}
