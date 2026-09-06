use super::{
    Arc, ChatEngine, ChatEvent, ConnectionId, Protocol, UserSession, info, mpsc,
    server_member_display_identity, warn,
};
use crate::engine::validation;

impl ChatEngine {
    /// Register a new session. Returns the session ID and an event receiver.
    pub fn connect(
        &self,
        user_id: Option<String>,
        nickname: String,
        protocol: Protocol,
        avatar_url: Option<String>,
    ) -> Result<(ConnectionId, mpsc::Receiver<ChatEvent>), String> {
        match protocol {
            Protocol::Irc => validation::validate_nickname(&nickname)?,
            Protocol::WebSocket => validation::validate_display_name(&nickname)?,
        }
        let nickname_key = crate::auth::authority::rfc1459_casefold(&nickname);

        // A user may connect from several clients with the same stable nickname.
        // The nickname remains exclusive across different user identities.
        if let Some(old_session_id) = self
            .nick_to_session
            .get(&nickname_key)
            .map(|entry| *entry.value())
        {
            let same_user = self
                .sessions
                .get(&old_session_id)
                .is_some_and(|session| user_id.is_some() && session.user_id == user_id);
            if !same_user {
                return Err(format!("Nickname already in use: {nickname}"));
            }
        }

        let session_id = ConnectionId::new();
        let (tx, rx) = mpsc::channel(crate::engine::user_session::MAX_OUTBOUND_QUEUE);

        let session = Arc::new(UserSession::new(
            session_id,
            user_id,
            nickname.clone(),
            protocol,
            tx,
            avatar_url,
        ));

        // Capture user_id before moving session into the map
        let session_user_id = session.user_id.clone();

        self.sessions.insert(session_id, session);
        self.nick_to_session.insert(nickname_key, session_id);
        if let Some(user_id) = &session_user_id {
            self.user_connections
                .entry(user_id.clone())
                .or_default()
                .insert(session_id);
        }

        // Update presence to online
        if let (Some(uid), Some(pool)) = (&session_user_id, &self.db) {
            let pool = pool.clone();
            let uid = uid.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::db::queries::presence::set_connected(&pool, &uid).await {
                    tracing::warn!(error = %e, "failed to update presence to online");
                }
            });
        }

        info!(%session_id, %nickname, ?protocol, "session connected");

        Ok((session_id, rx))
    }
    /// Disconnect a session and clean up all state.
    pub fn disconnect(&self, session_id: ConnectionId) {
        if let Some((_, actor)) = self.authenticated_actors.remove(&session_id)
            && let Some(mut connections) =
                self.credential_connections.get_mut(actor.credential_id())
        {
            connections.remove(&session_id);
            if connections.is_empty() {
                let credential_id = actor.credential_id().clone();
                drop(connections);
                self.credential_connections.remove(&credential_id);
            }
        }
        let Some((_, session)) = self.sessions.remove(&session_id) else {
            return;
        };

        let nickname = session.nickname.clone();
        if let Some(user_id) = &session.user_id
            && let Some(mut connections) = self.user_connections.get_mut(user_id)
        {
            connections.remove(&session_id);
            if connections.is_empty() {
                drop(connections);
                self.user_connections.remove(user_id);
            }
        }
        let nickname_key = crate::auth::authority::rfc1459_casefold(&nickname);
        if self
            .nick_to_session
            .get(&nickname_key)
            .is_some_and(|indexed| *indexed == session_id)
        {
            self.nick_to_session.remove(&nickname_key);
            if let Some(replacement) = self
                .sessions
                .iter()
                .find(|candidate| candidate.nickname == nickname)
                .map(|candidate| *candidate.key())
            {
                self.nick_to_session.insert(nickname_key, replacement);
            }
        }

        // Collect channels this session was in
        let channels_to_leave: Vec<String> = self
            .channels
            .iter()
            .filter(|ch| ch.members.contains(&session_id))
            .map(|ch| ch.key().clone())
            .collect();

        for channel_id in &channels_to_leave {
            if let Some(mut channel) = self.channels.get_mut(channel_id) {
                channel.members.remove(&session_id);
            }
        }

        let has_other_user_connection = session.user_id.as_ref().is_some_and(|uid| {
            self.user_connections
                .get(uid)
                .is_some_and(|connections| !connections.is_empty())
        });
        if !has_other_user_connection {
            let quit_event = ChatEvent::Quit {
                nickname: nickname.clone(),
                reason: None,
            };
            for channel_id in &channels_to_leave {
                self.broadcast_to_channel(channel_id, &quit_event, Some(session_id));
            }
        }

        // Update presence if this was the last session for this user
        if let Some(ref uid) = session.user_id {
            let other_sessions = self
                .user_connections
                .get(uid)
                .is_some_and(|connections| !connections.is_empty());
            if !other_sessions {
                if let Some(pool) = &self.db {
                    let _ = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(crate::db::queries::presence::set_offline(pool, uid))
                    });
                }
                // Broadcast offline using each server's durable display identity.
                let server_ids: Vec<String> = self
                    .servers
                    .iter()
                    .filter(|server| server.member_user_ids.contains(uid))
                    .map(|server| server.id.clone())
                    .collect();
                for server_id in server_ids {
                    let identity = self.db.as_ref().and_then(|pool| {
                        match tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(
                                server_member_display_identity(pool, &server_id, uid),
                            )
                        }) {
                            Ok(identity) => identity,
                            Err(error) => {
                                warn!(%error, %server_id, user_id = %uid, "offline presence identity query failed");
                                None
                            }
                        }
                    });
                    if let (Some((nickname, avatar_url)), Some(server)) =
                        (identity, self.servers.get(&server_id))
                    {
                        let event = ChatEvent::PresenceUpdate {
                            server_id: server_id.clone(),
                            presence: crate::engine::events::PresenceInfo {
                                user_id: uid.clone(),
                                nickname,
                                avatar_url,
                                status: "offline".into(),
                                custom_status: None,
                                status_emoji: None,
                            },
                        };
                        let mut notified = std::collections::HashSet::new();
                        for channel_id in server.channel_ids.iter() {
                            if let Some(channel) = self.channels.get(channel_id) {
                                for &member_sid in &channel.members {
                                    if member_sid != session_id
                                        && notified.insert(member_sid)
                                        && let Some(s) = self.sessions.get(&member_sid)
                                    {
                                        let _ = s.send(event.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(%session_id, %nickname, "session disconnected");
    }
}
