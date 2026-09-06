use super::{
    ChannelFollowInfo, ChatEngine, ChatEvent, ConnectionId, Permissions, normalize_channel_name,
    referenced_channel_id, referenced_server_id,
};

impl ChatEngine {
    /// Set a channel as an announcement channel. Requires MANAGE_CHANNELS permission.
    pub async fn set_announcement_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        is_announcement: bool,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        self.community_service()?
            .set_announcement(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                is_announcement,
            )
            .await
            .map_err(String::from)?;

        Ok(())
    }
    /// Follow an announcement channel, cross-posting to a target channel.
    /// Requires MANAGE_CHANNELS permission on the target server.
    pub async fn follow_channel(
        &self,
        session_id: ConnectionId,
        source_channel_id: &str,
        target_channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let created = self
            .community_service()?
            .follow_channel(
                &actor,
                &referenced_channel_id(source_channel_id)?,
                &referenced_channel_id(target_channel_id)?,
            )
            .await
            .map_err(String::from)?;

        let follow = ChannelFollowInfo {
            id: created.id,
            source_channel_id: source_channel_id.to_string(),
            target_channel_id: target_channel_id.to_string(),
            created_by: created.created_by,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ChannelFollowCreate { follow });
        }

        Ok(())
    }
    /// Unfollow an announcement channel. Requires MANAGE_CHANNELS permission.
    pub async fn unfollow_channel(
        &self,
        session_id: ConnectionId,
        follow_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let target_server = self
            .community_service()?
            .unfollow_channel(&actor, follow_id)
            .await
            .map_err(String::from)?;

        let session = self.get_session(session_id).ok_or("Session not found")?;
        let _ = session.send_guarded(
            ChatEvent::ChannelFollowDelete {
                follow_id: follow_id.to_string(),
            },
            Some(
                crate::engine::user_session::DeliveryGuard::ServerPermissions(vec![(
                    target_server.into_inner(),
                    Permissions::MANAGE_CHANNELS,
                )]),
            ),
        );

        Ok(())
    }
    /// List follows for an announcement channel. Sends ChannelFollowList to the session.
    pub async fn list_channel_follows(
        &self,
        session_id: ConnectionId,
        channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (rows, stamp) = self
            .community_service()?
            .list_channel_follows(&actor, &referenced_channel_id(channel_id)?)
            .await
            .map_err(String::from)?;

        let follows: Vec<ChannelFollowInfo> = rows
            .into_iter()
            .map(|r| ChannelFollowInfo {
                id: r.id,
                source_channel_id: r.source_channel_id,
                target_channel_id: r.target_channel_id,
                created_by: r.created_by,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::ChannelFollowList {
                    channel_id: channel_id.to_string(),
                    follows,
                },
                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                    stamp,
                ])),
            );
        }

        Ok(())
    }
    /// Explicitly publish a message from a public announcement channel.
    pub async fn publish_announcement(
        &self,
        session_id: ConnectionId,
        message_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let publications = self
            .messaging_service()
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?
            .publish_announcement(
                &actor,
                crate::engine::messaging::PublishAnnouncementCommand { message_id },
            )
            .await
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AnnouncementPublished {
                source_message_id: message_id.to_string(),
                published_count: publications.len(),
            });
        }
        Ok(())
    }
}
