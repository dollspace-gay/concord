use super::{
    ChannelState, ChatEngine, ChatEvent, ConnectionId, MemberInfo, Uuid, info,
    normalize_channel_name, referenced_channel_id, referenced_server_id,
};
use crate::engine::validation;

impl ChatEngine {
    /// Create a channel within a server. Returns the channel ID.
    pub async fn create_channel_in_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        category_id: Option<&str>,
        is_private: bool,
        channel_type: &str,
    ) -> Result<String, String> {
        if !matches!(channel_type, "text" | "forum") {
            return Err("channel type must be text or forum".into());
        }
        let name = normalize_channel_name(name);
        validation::validate_channel_name(&name)?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let channel_id = Uuid::new_v4().to_string();
        self.organization_service()?
            .create_channel(
                &actor,
                crate::engine::organization::CreateChannel {
                    server_id: &referenced_server_id(server_id)?,
                    channel_id: &referenced_channel_id(&channel_id)?,
                    name: &name,
                    category_id,
                    is_private,
                    channel_type,
                },
            )
            .await
            .map_err(String::from)?;

        let mut ch = ChannelState::new(channel_id.clone(), server_id.to_string(), name.clone());
        ch.category_id = category_id.map(|s| s.to_string());
        ch.is_private = is_private;
        ch.channel_type = channel_type.to_string();
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.insert(channel_id.clone());
        }
        self.channel_name_index
            .insert((server_id.to_string(), name.clone()), channel_id.clone());
        self.channels.insert(channel_id.clone(), ch);

        Ok(channel_id)
    }
    /// Delete a channel from a server.
    pub async fn delete_channel_in_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.organization_service()?
            .delete_channel(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
            )
            .await
            .map_err(String::from)?;

        self.channels.remove(&channel_id);
        self.channel_name_index
            .remove(&(server_id.to_string(), channel_name));
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.remove(&channel_id);
        }

        Ok(())
    }
    /// Join a channel within a server.
    pub async fn join_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        validation::validate_channel_name(&channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.clone()))
            .map(|id| id.clone())
            .ok_or_else(|| format!("No such channel: {channel_name}"))?;
        if let Some(pool) = &self.db {
            let actor = self
                .get_authenticated_actor(session_id)
                .ok_or_else(|| "resource unavailable".to_string())?;
            let auth = self.auth.get().ok_or("Authentication unavailable")?;
            crate::engine::authorization::AuthorizationService::new(pool.clone())
                .authorize_actor(
                    auth,
                    &actor,
                    &channel_id,
                    crate::engine::authorization::ChannelAction::View,
                )
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        }

        // Add session to channel
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            channel.members.insert(session_id);
        }

        // Copy the in-memory projection before database hydration so no DashMap
        // guard is held across an await.
        if let Some((topic, mut members)) = self.channels.get(&channel_id).map(|channel| {
            let members = channel
                .members
                .iter()
                .filter_map(|sid| {
                    self.sessions.get(sid).map(|s| MemberInfo {
                        nickname: s.nickname.clone(),
                        avatar_url: s.avatar_url.clone(),
                        server_avatar_url: None,
                        status: None,
                        custom_status: None,
                        status_emoji: None,
                        user_id: s.user_id.clone(),
                        role_ids: Vec::new(),
                    })
                })
                .collect::<Vec<_>>();
            (channel.topic.clone(), members)
        }) {
            if !topic.is_empty() {
                let _ = session.send_guarded(
                    ChatEvent::Topic {
                        server_id: server_id.to_string(),
                        channel: channel_name.clone(),
                        topic,
                    },
                    Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                        channel_id.clone(),
                    ])),
                );
            }

            // Send member list to the joiner. Hydrate server presentation and role
            // assignments here as well as in an explicit GetMembers response so a
            // reconnect cannot transiently erase the authoritative projection.
            if self.db.is_some() {
                self.hydrate_server_member_projections(server_id, &mut members)
                    .await?;
            }

            let joining_member = members
                .iter()
                .find(|member| {
                    session
                        .user_id
                        .as_deref()
                        .is_some_and(|user_id| member.user_id.as_deref() == Some(user_id))
                        || (session.user_id.is_none() && member.nickname == session.nickname)
                })
                .cloned()
                .unwrap_or(MemberInfo {
                    nickname: session.nickname.clone(),
                    avatar_url: session.avatar_url.clone(),
                    server_avatar_url: None,
                    status: None,
                    custom_status: None,
                    status_emoji: None,
                    user_id: session.user_id.clone(),
                    role_ids: Vec::new(),
                });
            self.broadcast_to_channel(
                &channel_id,
                &ChatEvent::Join {
                    nickname: joining_member.nickname,
                    server_id: server_id.to_string(),
                    channel: channel_name.clone(),
                    avatar_url: joining_member.avatar_url,
                    user_id: joining_member.user_id,
                    server_avatar_url: joining_member.server_avatar_url,
                    role_ids: joining_member.role_ids,
                },
                None,
            );

            let _ = session.send_guarded(
                ChatEvent::Names {
                    server_id: server_id.to_string(),
                    channel: channel_name.clone(),
                    members,
                },
                Some(crate::engine::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }

        info!(nickname = %session.nickname, %server_id, %channel_name, "joined channel");
        Ok(())
    }
    /// Leave a channel.
    pub fn part_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        reason: Option<String>,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let mut found = false;
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            found = channel.members.remove(&session_id);
        }

        if !found {
            return Err(format!("Not in channel {channel_name}"));
        }

        let part_event = ChatEvent::Part {
            nickname: session.nickname.clone(),
            server_id: server_id.to_string(),
            channel: channel_name.clone(),
            reason,
        };
        let _ = session.send(part_event.clone());
        self.broadcast_to_channel(&channel_id, &part_event, Some(session_id));

        // Remove empty channels from memory (but not from DB)
        self.channels
            .remove_if(&channel_id, |_, ch| ch.members.is_empty());

        info!(nickname = %session.nickname, %server_id, %channel_name, "parted channel");
        Ok(())
    }
}
