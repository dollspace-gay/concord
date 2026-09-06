use super::{
    AuditLogEntry, BanInfo, ChatEngine, ChatEvent, ConnectionId, moderation_unauthenticated,
    moderation_unavailable, normalize_channel_name, referenced_channel_id, referenced_server_id,
};

impl ChatEngine {
    /// Broadcast a ChatEvent to all connected sessions that belong to a server.
    pub fn broadcast_to_server(&self, server_id: &str, event: &ChatEvent) {
        let Some(server) = self.servers.get(server_id) else {
            return;
        };
        let member_ids: Vec<String> = server.member_user_ids.iter().cloned().collect();
        drop(server);

        for session in self.sessions.iter() {
            if let Some(uid) = &session.user_id
                && member_ids.contains(uid)
            {
                let _ = session.send(event.clone());
            }
        }
    }
    /// Kick a member from a server.
    pub async fn kick_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(), String> {
        self.kick_member_in_channel(session_id, server_id, target_user_id, reason, None)
            .await
    }
    /// Kick a member with channel-scoped permission check.
    /// When `channel_id` is Some, the permission check considers channel overrides,
    /// allowing moderators with per-channel KICK_MEMBERS to kick from that channel.
    pub async fn kick_member_in_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
        channel_id: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let server_resource_id = referenced_server_id(server_id)?;
        let channel_resource_id = channel_id.map(referenced_channel_id).transpose()?;
        self.moderation_service()?
            .kick_member(
                &actor,
                &server_resource_id,
                target_user_id,
                reason,
                channel_resource_id.as_ref(),
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Remove from in-memory server state
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(target_user_id);
        }
        self.evict_user_from_server_subscriptions(server_id, target_user_id);

        // Broadcast kick event to server members
        let event = ChatEvent::MemberKick {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            kicked_by: actor.user_id().as_str().to_owned(),
            reason: reason.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Ban a member from a server, optionally deleting their messages.
    pub async fn ban_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
        delete_message_days: i32,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .ban_member(
                &actor,
                &referenced_server_id(server_id)?,
                target_user_id,
                reason,
                delete_message_days,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Remove from in-memory server state
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(target_user_id);
        }
        self.evict_user_from_server_subscriptions(server_id, target_user_id);

        // Broadcast
        let event = ChatEvent::MemberBan {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            banned_by: actor.user_id().as_str().to_owned(),
            reason: reason.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Unban a member from a server.
    pub async fn unban_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .unban_member(&actor, &referenced_server_id(server_id)?, target_user_id)
            .await
            .map_err(|error| error.wire_message())?;

        // Broadcast
        let event = ChatEvent::MemberUnban {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Get the list of bans for a server.
    pub async fn list_bans(&self, session_id: ConnectionId, server_id: &str) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rows = self
            .moderation_service()?
            .list_bans(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(|error| error.wire_message())?;

        let bans: Vec<BanInfo> = rows
            .into_iter()
            .map(|r| BanInfo {
                id: r.id,
                user_id: r.user_id,
                banned_by: r.banned_by,
                reason: r.reason,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BanList {
                server_id: server_id.to_string(),
                bans,
            });
        }

        Ok(())
    }
    /// Set a timeout on a member (or clear it).
    pub async fn timeout_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        timeout_until: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .timeout_member(
                &actor,
                &referenced_server_id(server_id)?,
                target_user_id,
                timeout_until,
                reason,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Broadcast
        let event = ChatEvent::MemberTimeout {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            timeout_until: timeout_until.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Set slow mode on a channel.
    pub async fn set_slowmode(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        seconds: i32,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|v| v.clone())
            .ok_or_else(moderation_unavailable)?;
        self.moderation_service()?
            .set_slowmode(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                seconds,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Update in-memory state
        if let Some(mut ch) = self.channels.get_mut(&channel_id) {
            ch.slowmode_seconds = seconds;
        }

        // Broadcast
        let event = ChatEvent::SlowModeUpdate {
            server_id: server_id.to_string(),
            channel: channel_name.to_string(),
            seconds,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Set NSFW flag on a channel.
    pub async fn set_nsfw(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        is_nsfw: bool,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|v| v.clone())
            .ok_or_else(moderation_unavailable)?;
        self.moderation_service()?
            .set_nsfw(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                is_nsfw,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Update in-memory state
        if let Some(mut ch) = self.channels.get_mut(&channel_id) {
            ch.is_nsfw = is_nsfw;
        }

        // Broadcast
        let event = ChatEvent::NsfwUpdate {
            server_id: server_id.to_string(),
            channel: channel_name.to_string(),
            is_nsfw,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Bulk delete messages in a channel (up to 100).
    pub async fn bulk_delete_messages(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_ids: Vec<String>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let channel_id =
            self.resolve_channel_id(server_id, &normalize_channel_name(channel_name))?;
        self.moderation_service()?
            .bulk_delete_messages(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                &message_ids,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Broadcast
        let event = ChatEvent::BulkMessageDelete {
            server_id: server_id.to_string(),
            channel: channel_name.to_string(),
            message_ids,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }
    /// Get audit log entries for a server.
    pub async fn get_audit_log(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        action_type: Option<&str>,
        limit: i64,
        before: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rows = self
            .moderation_service()?
            .list_audit_log(
                &actor,
                &referenced_server_id(server_id)?,
                action_type,
                limit,
                before,
            )
            .await
            .map_err(|error| error.wire_message())?;

        let entries: Vec<AuditLogEntry> = rows
            .into_iter()
            .map(|r| AuditLogEntry {
                id: r.id,
                actor_id: r.actor_id,
                actor_username_snapshot: r.actor_username_snapshot,
                actor_avatar_snapshot: r.actor_avatar_snapshot,
                action_type: r.action_type,
                target_type: r.target_type,
                target_id: r.target_id,
                reason: r.reason,
                changes: r.changes,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AuditLogEntries {
                server_id: server_id.to_string(),
                entries,
            });
        }

        Ok(())
    }
}
