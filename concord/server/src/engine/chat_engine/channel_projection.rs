use super::{
    ChannelInfo, ChatEngine, ChatEvent, ConnectionId, MemberInfo, channel_conversation_id,
    normalize_channel_name,
};

impl ChatEngine {
    /// List all channels in a server.
    pub fn list_channels(&self, server_id: &str) -> Vec<ChannelInfo> {
        self.channels
            .iter()
            .filter(|ch| ch.server_id == server_id)
            .map(|entry| ChannelInfo {
                id: entry.id.clone(),
                conversation_id: channel_conversation_id(&entry.id),
                server_id: entry.server_id.clone(),
                name: entry.name.clone(),
                topic: entry.topic.clone(),
                member_count: entry.member_count(),
                category_id: entry.category_id.clone(),
                position: entry.position,
                is_private: entry.is_private,
                channel_type: entry.channel_type.clone(),
                thread_parent_message_id: entry.thread_parent_message_id.clone(),
                archived: entry.archived,
                slowmode_seconds: entry.slowmode_seconds,
                is_nsfw: entry.is_nsfw,
            })
            .collect()
    }
    /// List only channels visible to the current database-backed member snapshot.
    pub async fn list_visible_channels(
        &self,
        server_id: &str,
        user_id: &str,
    ) -> Result<Vec<ChannelInfo>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = crate::engine::authorization::AuthorizationService::new(pool.clone())
            .visible_channels(user_id, server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        Ok(rows
            .into_iter()
            .map(|row| ChannelInfo {
                member_count: self
                    .channels
                    .get(&row.id)
                    .map_or(0, |state| state.member_count()),
                conversation_id: channel_conversation_id(&row.id),
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                topic: row.topic,
                category_id: row.category_id,
                position: row.position,
                is_private: row.is_private != 0,
                channel_type: row.channel_type,
                thread_parent_message_id: row.thread_parent_message_id,
                archived: row.archived != 0,
                slowmode_seconds: row.slowmode_seconds,
                is_nsfw: row.is_nsfw != 0,
            })
            .collect())
    }
    pub async fn list_visible_channels_for_actor(
        &self,
        server_id: &str,
        actor: &crate::auth::authority::Actor,
    ) -> Result<
        (
            Vec<ChannelInfo>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let (rows, stamp) = crate::engine::authorization::AuthorizationService::new(pool.clone())
            .visible_channels_for_actor(auth, actor, server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let channels = rows
            .into_iter()
            .map(|row| ChannelInfo {
                member_count: self
                    .channels
                    .get(&row.id)
                    .map_or(0, |state| state.member_count()),
                conversation_id: channel_conversation_id(&row.id),
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                topic: row.topic,
                category_id: row.category_id,
                position: row.position,
                is_private: row.is_private != 0,
                channel_type: row.channel_type,
                thread_parent_message_id: row.thread_parent_message_id,
                archived: row.archived != 0,
                slowmode_seconds: row.slowmode_seconds,
                is_nsfw: row.is_nsfw != 0,
            })
            .collect();
        Ok((channels, stamp))
    }
    pub async fn send_visible_channel_list(
        &self,
        session_id: ConnectionId,
        server_id: String,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (channels, stamp) = self
            .list_visible_channels_for_actor(&server_id, &actor)
            .await?;
        let session = self
            .get_session(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        if !session.send_guarded(
            ChatEvent::ChannelList {
                server_id,
                channels,
            },
            Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                stamp,
            ])),
        ) {
            return Err("delivery unavailable".into());
        }
        Ok(())
    }
    /// Get members of a channel.
    pub fn get_members(
        &self,
        server_id: &str,
        channel_name: &str,
    ) -> Result<Vec<MemberInfo>, String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let channel = self
            .channels
            .get(&channel_id)
            .ok_or(format!("No such channel: {channel_name}"))?;

        Ok(channel
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
            .collect())
    }
    pub async fn get_visible_members(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        channel_name: &str,
    ) -> Result<
        (
            Vec<MemberInfo>,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let normalized = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &normalized)?;
        let stamp = crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_stamped(
                auth,
                actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::View,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let mut members = self.get_members(server_id, &normalized)?;
        self.hydrate_server_member_projections(server_id, &mut members)
            .await?;
        Ok((members, stamp))
    }
    pub(super) async fn hydrate_server_member_projections(
        &self,
        server_id: &str,
        members: &mut [MemberInfo],
    ) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let identities: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT user_id,nickname,avatar_url FROM server_members WHERE server_id=?",
        )
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let identities: std::collections::HashMap<_, _> = identities
            .into_iter()
            .map(|(user_id, nickname, avatar)| (user_id, (nickname, avatar)))
            .collect();
        let role_rows: Vec<(String, String)> =
            sqlx::query_as("SELECT user_id,role_id FROM user_roles WHERE server_id=?")
                .bind(server_id)
                .fetch_all(pool)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        let mut role_ids: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (user_id, role_id) in role_rows {
            role_ids.entry(user_id).or_default().push(role_id);
        }
        for member in members {
            if let Some(user_id) = member.user_id.as_deref()
                && let Some((nickname, avatar)) = identities.get(user_id)
            {
                if let Some(nickname) = nickname {
                    member.nickname.clone_from(nickname);
                }
                member.server_avatar_url.clone_from(avatar);
            }
            if let Some(user_id) = member.user_id.as_deref() {
                member.role_ids = role_ids.remove(user_id).unwrap_or_default();
            }
        }
        Ok(())
    }
}
