use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::channel::ChannelState;
use super::events::{ChannelInfo, ChatEvent, HistoryMessage, MemberInfo, ServerInfo, SessionId};
use super::permissions::ServerRole;
use super::rate_limiter::RateLimiter;
use super::server::ServerState;
use super::user_session::{Protocol, UserSession};
use super::validation;

/// The default server ID used as a fallback for IRC clients
/// that don't specify a server. No server with this ID is pre-created;
/// IRC bare-channel operations will fail unless one is created by a user.
pub const DEFAULT_SERVER_ID: &str = "default";

/// The central hub that manages all chat state. Protocol-agnostic —
/// both IRC and WebSocket adapters call into this.
pub struct ChatEngine {
    /// All currently connected sessions, keyed by session ID.
    sessions: DashMap<SessionId, Arc<UserSession>>,
    /// All servers (guilds), keyed by server ID.
    servers: DashMap<String, ServerState>,
    /// All channels, keyed by channel UUID.
    channels: DashMap<String, ChannelState>,
    /// Index: (server_id, channel_name) -> channel_id for name-based lookups.
    channel_name_index: DashMap<(String, String), String>,
    /// Reverse lookup: nickname -> session ID (for DMs and WHOIS).
    nick_to_session: DashMap<String, SessionId>,
    /// Optional database pool. When present, messages and channels are persisted.
    db: Option<SqlitePool>,
    /// Per-user message rate limiter (burst of 10, refill 1 per second).
    message_limiter: RateLimiter,
}

impl ChatEngine {
    pub fn new(db: Option<SqlitePool>) -> Self {
        Self {
            sessions: DashMap::new(),
            servers: DashMap::new(),
            channels: DashMap::new(),
            channel_name_index: DashMap::new(),
            nick_to_session: DashMap::new(),
            db,
            message_limiter: RateLimiter::new(10, 1.0),
        }
    }

    // ── Startup loading ─────────────────────────────────────────────

    /// Load servers from the database into memory on startup.
    pub async fn load_servers_from_db(&self) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Ok(());
        };

        let rows = crate::db::queries::servers::list_all_servers(pool)
            .await
            .map_err(|e| format!("Failed to load servers: {e}"))?;

        for row in rows {
            let mut state = ServerState::new(row.id.clone(), row.name, row.owner_id, row.icon_url);

            let members = crate::db::queries::servers::get_server_members(pool, &row.id)
                .await
                .map_err(|e| format!("Failed to load server members: {e}"))?;
            for m in members {
                state.member_user_ids.insert(m.user_id);
            }

            self.servers.insert(row.id, state);
        }

        info!(count = self.servers.len(), "loaded servers from database");
        Ok(())
    }

    /// Load channels from the database into memory on startup.
    pub async fn load_channels_from_db(&self) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Ok(());
        };

        // Collect server IDs first to avoid holding a read lock on self.servers
        // while later acquiring a write lock via get_mut (DashMap deadlock).
        let server_ids: Vec<String> = self.servers.iter().map(|s| s.id.clone()).collect();

        for server_id in &server_ids {
            let rows = crate::db::queries::channels::list_channels(pool, server_id)
                .await
                .map_err(|e| format!("Failed to load channels: {e}"))?;

            for row in rows {
                let mut ch =
                    ChannelState::new(row.id.clone(), row.server_id.clone(), row.name.clone());
                ch.topic = row.topic;
                ch.topic_set_by = row.topic_set_by;

                self.channel_name_index
                    .insert((row.server_id.clone(), row.name), row.id.clone());

                if let Some(mut srv) = self.servers.get_mut(&row.server_id) {
                    srv.channel_ids.insert(row.id.clone());
                }

                self.channels.insert(row.id, ch);
            }
        }

        info!(count = self.channels.len(), "loaded channels from database");
        Ok(())
    }

    // ── Session management ──────────────────────────────────────────

    /// Register a new session. Returns the session ID and an event receiver.
    pub fn connect(
        &self,
        user_id: Option<String>,
        nickname: String,
        protocol: Protocol,
        avatar_url: Option<String>,
    ) -> Result<(SessionId, mpsc::UnboundedReceiver<ChatEvent>), String> {
        validation::validate_nickname(&nickname)?;

        // If nickname is already in use, disconnect the stale session.
        if let Some(old_session_id) = self.nick_to_session.get(&nickname).map(|r| *r) {
            info!(%nickname, "replacing stale session for reconnecting user");
            self.disconnect(old_session_id);
        }

        let session_id = Uuid::new_v4();
        let (tx, rx) = mpsc::unbounded_channel();

        let session = Arc::new(UserSession::new(
            session_id,
            user_id,
            nickname.clone(),
            protocol,
            tx,
            avatar_url,
        ));

        self.sessions.insert(session_id, session);
        self.nick_to_session.insert(nickname.clone(), session_id);

        info!(%session_id, %nickname, ?protocol, "session connected");

        Ok((session_id, rx))
    }

    /// Disconnect a session and clean up all state.
    pub fn disconnect(&self, session_id: SessionId) {
        let Some((_, session)) = self.sessions.remove(&session_id) else {
            return;
        };

        let nickname = session.nickname.clone();
        self.nick_to_session.remove(&nickname);

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

        // Broadcast quit to all channels this user was in
        let quit_event = ChatEvent::Quit {
            nickname: nickname.clone(),
            reason: None,
        };

        for channel_id in &channels_to_leave {
            self.broadcast_to_channel(channel_id, &quit_event, Some(session_id));
        }

        info!(%session_id, %nickname, "session disconnected");
    }

    // ── Server management ───────────────────────────────────────────

    /// Create a new server. Returns the server ID.
    pub async fn create_server(
        &self,
        name: String,
        owner_user_id: String,
        icon_url: Option<String>,
    ) -> Result<String, String> {
        validation::validate_server_name(&name)?;

        let server_id = Uuid::new_v4().to_string();

        if let Some(pool) = &self.db {
            crate::db::queries::servers::create_server(
                pool,
                &server_id,
                &name,
                &owner_user_id,
                icon_url.as_deref(),
            )
            .await
            .map_err(|e| format!("Failed to create server: {e}"))?;
        }

        let mut state = ServerState::new(
            server_id.clone(),
            name.clone(),
            owner_user_id.clone(),
            icon_url,
        );
        state.member_user_ids.insert(owner_user_id);
        self.servers.insert(server_id.clone(), state);

        // Create default #general channel
        let channel_id = Uuid::new_v4().to_string();
        let channel_name = "#general".to_string();
        if let Some(pool) = &self.db {
            let _ = crate::db::queries::channels::ensure_channel(
                pool,
                &channel_id,
                &server_id,
                &channel_name,
            )
            .await;
        }
        let ch = ChannelState::new(channel_id.clone(), server_id.clone(), channel_name.clone());
        self.channel_name_index
            .insert((server_id.clone(), channel_name), channel_id.clone());
        if let Some(mut srv) = self.servers.get_mut(&server_id) {
            srv.channel_ids.insert(channel_id.clone());
        }
        self.channels.insert(channel_id, ch);

        info!(%server_id, %name, "server created");
        Ok(server_id)
    }

    /// Delete a server.
    pub async fn delete_server(&self, server_id: &str) -> Result<(), String> {
        if let Some(server) = self.servers.get(server_id) {
            for ch_id in &server.channel_ids {
                if let Some((_, ch)) = self.channels.remove(ch_id) {
                    self.channel_name_index
                        .remove(&(server_id.to_string(), ch.name));
                }
            }
        }

        self.servers.remove(server_id);

        if let Some(pool) = &self.db {
            crate::db::queries::servers::delete_server(pool, server_id)
                .await
                .map_err(|e| format!("Failed to delete server: {e}"))?;
        }

        info!(%server_id, "server deleted");
        Ok(())
    }

    /// List servers for a user (by their DB user_id).
    pub fn list_servers_for_user(&self, user_id: &str) -> Vec<ServerInfo> {
        self.servers
            .iter()
            .filter(|s| s.member_user_ids.contains(user_id))
            .map(|s| {
                let role = if s.owner_id == user_id {
                    Some("owner".to_string())
                } else {
                    Some("member".to_string())
                };
                ServerInfo {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    icon_url: s.icon_url.clone(),
                    member_count: s.member_user_ids.len(),
                    role,
                }
            })
            .collect()
    }

    /// List all servers (for system admin).
    pub fn list_all_servers(&self) -> Vec<ServerInfo> {
        self.servers
            .iter()
            .map(|s| ServerInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                icon_url: s.icon_url.clone(),
                member_count: s.member_user_ids.len(),
                role: None,
            })
            .collect()
    }

    /// Check if a user is the owner of a server.
    pub fn is_server_owner(&self, server_id: &str, user_id: &str) -> bool {
        self.servers
            .get(server_id)
            .map(|s| s.owner_id == user_id)
            .unwrap_or(false)
    }

    /// Join a server (persistent membership).
    pub async fn join_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if !self.servers.contains_key(server_id) {
            return Err(format!("No such server: {server_id}"));
        }

        if let Some(pool) = &self.db {
            crate::db::queries::servers::add_server_member(pool, server_id, user_id, "member")
                .await
                .map_err(|e| format!("Failed to join server: {e}"))?;
        }

        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.insert(user_id.to_string());
        }

        Ok(())
    }

    /// Leave a server (remove persistent membership).
    pub async fn leave_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if let Some(pool) = &self.db {
            crate::db::queries::servers::remove_server_member(pool, server_id, user_id)
                .await
                .map_err(|e| format!("Failed to leave server: {e}"))?;
        }

        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(user_id);
        }

        Ok(())
    }

    /// Get the role of a user in a server.
    pub async fn get_server_role(&self, server_id: &str, user_id: &str) -> Option<ServerRole> {
        let Some(pool) = &self.db else {
            return None;
        };
        let member = crate::db::queries::servers::get_server_member(pool, server_id, user_id)
            .await
            .ok()
            .flatten()?;
        Some(ServerRole::parse(&member.role))
    }

    /// Look up server_id by server name (for IRC).
    pub fn find_server_by_name(&self, name: &str) -> Option<String> {
        let name_lower = name.to_lowercase();
        self.servers
            .iter()
            .find(|s| s.name.to_lowercase() == name_lower)
            .map(|s| s.id.clone())
    }

    /// Get a server's name by ID.
    pub fn get_server_name(&self, server_id: &str) -> Option<String> {
        self.servers.get(server_id).map(|s| s.name.clone())
    }

    // ── Channel management ──────────────────────────────────────────

    /// Create a channel within a server. Returns the channel ID.
    pub async fn create_channel_in_server(
        &self,
        server_id: &str,
        name: &str,
    ) -> Result<String, String> {
        let name = normalize_channel_name(name);
        validation::validate_channel_name(&name)?;

        if !self.servers.contains_key(server_id) {
            return Err(format!("No such server: {server_id}"));
        }

        if self
            .channel_name_index
            .contains_key(&(server_id.to_string(), name.clone()))
        {
            return Err(format!("Channel {name} already exists in this server"));
        }

        let channel_id = Uuid::new_v4().to_string();

        if let Some(pool) = &self.db {
            crate::db::queries::channels::ensure_channel(pool, &channel_id, server_id, &name)
                .await
                .map_err(|e| format!("Failed to create channel: {e}"))?;
        }

        let ch = ChannelState::new(channel_id.clone(), server_id.to_string(), name.clone());
        self.channel_name_index
            .insert((server_id.to_string(), name), channel_id.clone());
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.insert(channel_id.clone());
        }
        self.channels.insert(channel_id.clone(), ch);

        Ok(channel_id)
    }

    /// Delete a channel from a server.
    pub async fn delete_channel_in_server(
        &self,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        if let Some(pool) = &self.db {
            crate::db::queries::channels::delete_channel(pool, &channel_id)
                .await
                .map_err(|e| format!("Failed to delete channel: {e}"))?;
        }

        self.channels.remove(&channel_id);
        self.channel_name_index
            .remove(&(server_id.to_string(), channel_name));
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.remove(&channel_id);
        }

        Ok(())
    }

    /// Join a channel within a server.
    pub fn join_channel(
        &self,
        session_id: SessionId,
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

        // Get or create channel
        let channel_id = if let Some(id) = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.clone()))
        {
            id.clone()
        } else {
            // Create channel on-the-fly
            let new_id = Uuid::new_v4().to_string();
            let ch = ChannelState::new(new_id.clone(), server_id.to_string(), channel_name.clone());
            self.channels.insert(new_id.clone(), ch);
            self.channel_name_index.insert(
                (server_id.to_string(), channel_name.clone()),
                new_id.clone(),
            );
            if let Some(mut srv) = self.servers.get_mut(server_id) {
                srv.channel_ids.insert(new_id.clone());
            }

            // Persist channel to database
            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let ch_id = new_id.clone();
                let srv_id = server_id.to_string();
                let ch_name = channel_name.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::db::queries::channels::ensure_channel(
                        &pool, &ch_id, &srv_id, &ch_name,
                    )
                    .await
                    {
                        error!(error = %e, "failed to persist channel");
                    }
                });
            }

            new_id
        };

        // Add session to channel
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            channel.members.insert(session_id);
        }

        // Broadcast join event
        let join_event = ChatEvent::Join {
            nickname: session.nickname.clone(),
            server_id: server_id.to_string(),
            channel: channel_name.clone(),
            avatar_url: session.avatar_url.clone(),
        };
        self.broadcast_to_channel(&channel_id, &join_event, None);

        // Send current topic to the joiner
        if let Some(channel) = self.channels.get(&channel_id) {
            if !channel.topic.is_empty() {
                let _ = session.send(ChatEvent::Topic {
                    server_id: server_id.to_string(),
                    channel: channel_name.clone(),
                    topic: channel.topic.clone(),
                });
            }

            // Send member list to the joiner
            let members: Vec<MemberInfo> = channel
                .members
                .iter()
                .filter_map(|sid| {
                    self.sessions.get(sid).map(|s| MemberInfo {
                        nickname: s.nickname.clone(),
                        avatar_url: s.avatar_url.clone(),
                    })
                })
                .collect();

            let _ = session.send(ChatEvent::Names {
                server_id: server_id.to_string(),
                channel: channel_name.clone(),
                members,
            });
        }

        info!(nickname = %session.nickname, %server_id, %channel_name, "joined channel");
        Ok(())
    }

    /// Leave a channel.
    pub fn part_channel(
        &self,
        session_id: SessionId,
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

    /// Send a message to a channel or user (DM).
    pub fn send_message(
        &self,
        session_id: SessionId,
        server_id: &str,
        target: &str,
        content: &str,
    ) -> Result<(), String> {
        validation::validate_message(content)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        if !self.message_limiter.check(&session.nickname) {
            return Err("Rate limit exceeded. Please slow down.".into());
        }

        let msg_id = Uuid::new_v4();
        let event = ChatEvent::Message {
            id: msg_id,
            server_id: Some(server_id.to_string()),
            from: session.nickname.clone(),
            target: target.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            avatar_url: session.avatar_url.clone(),
        };

        if target.starts_with('#') {
            let channel_name = normalize_channel_name(target);
            let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

            let channel = self
                .channels
                .get(&channel_id)
                .ok_or(format!("No such channel: {channel_name}"))?;

            if !channel.members.contains(&session_id) {
                return Err(format!("You are not in channel {channel_name}"));
            }

            drop(channel);

            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let id = msg_id.to_string();
                let srv = server_id.to_string();
                let ch = channel_id.clone();
                let sid = session_id.to_string();
                let nick = session.nickname.clone();
                let msg = content.to_string();
                tokio::spawn(async move {
                    if let Err(e) = crate::db::queries::messages::insert_message(
                        &pool, &id, &srv, &ch, &sid, &nick, &msg,
                    )
                    .await
                    {
                        error!(error = %e, "failed to persist message");
                    }
                });
            }

            self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        } else {
            // DM
            let target_session_id = self
                .nick_to_session
                .get(target)
                .ok_or(format!("No such user: {target}"))?;

            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let id = msg_id.to_string();
                let sid = session_id.to_string();
                let nick = session.nickname.clone();
                let target_sid = target_session_id.value().to_string();
                let msg = content.to_string();
                tokio::spawn(async move {
                    if let Err(e) = crate::db::queries::messages::insert_dm(
                        &pool,
                        &id,
                        &sid,
                        &nick,
                        &target_sid,
                        &msg,
                    )
                    .await
                    {
                        error!(error = %e, "failed to persist DM");
                    }
                });
            }

            if let Some(target_session) = self.sessions.get(target_session_id.value()) {
                let _ = target_session.send(event);
            }
        }

        Ok(())
    }

    /// Set the topic for a channel.
    pub fn set_topic(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        topic: String,
    ) -> Result<(), String> {
        validation::validate_topic(&topic)?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let mut channel = self
            .channels
            .get_mut(&channel_id)
            .ok_or(format!("No such channel: {channel_name}"))?;

        if !channel.members.contains(&session_id) {
            return Err(format!("You are not in channel {channel_name}"));
        }

        channel.topic.clone_from(&topic);
        channel.topic_set_by = Some(session.nickname.clone());
        channel.topic_set_at = Some(Utc::now());

        drop(channel);

        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let ch = channel_id.clone();
            let t = topic.clone();
            let by = session.nickname.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::db::queries::channels::set_topic(&pool, &ch, &t, &by).await {
                    error!(error = %e, "failed to persist topic");
                }
            });
        }

        let event = ChatEvent::TopicChange {
            server_id: server_id.to_string(),
            channel: channel_name,
            set_by: session.nickname.clone(),
            topic,
        };
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Fetch message history for a channel.
    pub async fn fetch_history(
        &self,
        server_id: &str,
        channel_name: &str,
        before: Option<&str>,
        limit: i64,
    ) -> Result<(Vec<HistoryMessage>, bool), String> {
        let Some(pool) = &self.db else {
            return Ok((vec![], false));
        };

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let rows = crate::db::queries::messages::fetch_channel_history(
            pool,
            &channel_id,
            before,
            limit + 1,
        )
        .await
        .map_err(|e| format!("Failed to fetch history: {e}"))?;

        let has_more = rows.len() as i64 > limit;
        let messages: Vec<HistoryMessage> = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| HistoryMessage {
                id: row.id.parse().unwrap_or_default(),
                from: row.sender_nick,
                content: row.content,
                timestamp: row.created_at.parse().unwrap_or_else(|_| Utc::now()),
            })
            .collect();

        Ok((messages, has_more))
    }

    /// List all channels in a server.
    pub fn list_channels(&self, server_id: &str) -> Vec<ChannelInfo> {
        self.channels
            .iter()
            .filter(|ch| ch.server_id == server_id)
            .map(|entry| ChannelInfo {
                id: entry.id.clone(),
                server_id: entry.server_id.clone(),
                name: entry.name.clone(),
                topic: entry.topic.clone(),
                member_count: entry.member_count(),
            })
            .collect()
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
                })
            })
            .collect())
    }

    // ── Utility ─────────────────────────────────────────────────────

    /// Get a reference to the database pool (if configured).
    pub fn db(&self) -> Option<&SqlitePool> {
        self.db.as_ref()
    }

    /// Check if a nickname is available.
    pub fn is_nick_available(&self, nickname: &str) -> bool {
        !self.nick_to_session.contains_key(nickname)
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: SessionId) -> Option<Arc<UserSession>> {
        self.sessions.get(&session_id).map(|s| s.clone())
    }

    /// Resolve a channel name within a server to its channel ID.
    pub fn resolve_channel_id(
        &self,
        server_id: &str,
        channel_name: &str,
    ) -> Result<String, String> {
        self.channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|r| r.clone())
            .ok_or(format!("No such channel: {channel_name}"))
    }

    /// Broadcast an event to all members of a channel, optionally excluding one session.
    fn broadcast_to_channel(
        &self,
        channel_id: &str,
        event: &ChatEvent,
        exclude: Option<SessionId>,
    ) {
        let Some(channel) = self.channels.get(channel_id) else {
            return;
        };

        for member_id in &channel.members {
            if Some(*member_id) == exclude {
                continue;
            }
            if let Some(session) = self.sessions.get(member_id)
                && !session.send(event.clone())
            {
                warn!(%member_id, "failed to send event to session (channel closed)");
            }
        }
    }
}

/// Ensure channel names are lowercase and start with #.
fn normalize_channel_name(name: &str) -> String {
    let name = name.to_lowercase();
    if name.starts_with('#') {
        name
    } else {
        format!("#{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_channel_name() {
        assert_eq!(normalize_channel_name("#General"), "#general");
        assert_eq!(normalize_channel_name("general"), "#general");
        assert_eq!(normalize_channel_name("#rust"), "#rust");
    }

    /// Helper: create engine with a default server in memory (no DB).
    fn setup_engine() -> ChatEngine {
        let engine = ChatEngine::new(None);
        let state = ServerState::new(
            DEFAULT_SERVER_ID.to_string(),
            "Concord".to_string(),
            "system".to_string(),
            None,
        );
        engine.servers.insert(DEFAULT_SERVER_ID.to_string(), state);
        engine
    }

    #[tokio::test]
    async fn test_connect_and_disconnect() {
        let engine = setup_engine();

        let (session_id, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        assert!(!engine.is_nick_available("alice"));

        engine.disconnect(session_id);
        assert!(engine.is_nick_available("alice"));
    }

    #[tokio::test]
    async fn test_duplicate_nick_replaces_old_session() {
        let engine = setup_engine();

        let (sid1, _rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid2, _rx2) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();

        assert!(engine.get_session(sid1).is_none());
        assert!(engine.get_session(sid2).is_some());
    }

    #[tokio::test]
    async fn test_join_and_message() {
        let engine = setup_engine();

        let (sid1, mut rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid2, mut rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid1, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        engine
            .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
            .unwrap();

        while rx1.try_recv().is_ok() {}
        while rx2.try_recv().is_ok() {}

        engine
            .send_message(sid1, DEFAULT_SERVER_ID, "#general", "Hello from Alice!")
            .unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message { from, content, .. } => {
                assert_eq!(from, "alice");
                assert_eq!(content, "Hello from Alice!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }

        assert!(rx1.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_part_channel() {
        let engine = setup_engine();

        let (sid1, mut rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid2, _rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid1, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        engine
            .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
            .unwrap();

        while rx1.try_recv().is_ok() {}

        engine
            .part_channel(sid2, DEFAULT_SERVER_ID, "#general", None)
            .unwrap();

        let event = rx1.try_recv().unwrap();
        match event {
            ChatEvent::Part { nickname, .. } => assert_eq!(nickname, "bob"),
            _ => panic!("Expected Part event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_set_topic() {
        let engine = setup_engine();

        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        while rx.try_recv().is_ok() {}

        engine
            .set_topic(
                sid,
                DEFAULT_SERVER_ID,
                "#general",
                "Welcome to Concord!".into(),
            )
            .unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            ChatEvent::TopicChange { topic, .. } => {
                assert_eq!(topic, "Welcome to Concord!");
            }
            _ => panic!("Expected TopicChange event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_dm() {
        let engine = setup_engine();

        let (sid1, _rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (_sid2, mut rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .send_message(sid1, DEFAULT_SERVER_ID, "bob", "Hey Bob!")
            .unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message {
                from,
                target,
                content,
                ..
            } => {
                assert_eq!(from, "alice");
                assert_eq!(target, "bob");
                assert_eq!(content, "Hey Bob!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }
    }

    #[test]
    fn test_list_channels() {
        let engine = setup_engine();

        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#rust")
            .unwrap();

        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert_eq!(channels.len(), 2);

        let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"#general"));
        assert!(names.contains(&"#rust"));
    }

    #[tokio::test]
    async fn test_create_server() {
        let engine = setup_engine();

        let server_id = engine
            .create_server("Test Server".into(), "user1".into(), None)
            .await
            .unwrap();

        assert!(engine.servers.contains_key(&server_id));
        let channels = engine.list_channels(&server_id);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].name, "#general");
    }

    #[tokio::test]
    async fn test_server_isolation() {
        let engine = setup_engine();

        let server_a = engine
            .create_server("Server A".into(), "user1".into(), None)
            .await
            .unwrap();
        let server_b = engine
            .create_server("Server B".into(), "user1".into(), None)
            .await
            .unwrap();

        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();

        engine.join_channel(sid, &server_a, "#general").unwrap();
        while rx.try_recv().is_ok() {}

        let (sid2, _rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();
        engine.join_channel(sid2, &server_b, "#general").unwrap();

        // Alice is not in server_b's #general — should fail
        let result = engine.send_message(sid, &server_b, "#general", "Hello");
        assert!(result.is_err());
    }
}
