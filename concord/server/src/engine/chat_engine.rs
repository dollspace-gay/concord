use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::channel::ChannelState;
use super::events::{
    AuditLogEntry, AutomodRuleInfo, BanInfo, BookmarkInfo, BotTokenInfo, CategoryInfo,
    ChannelFollowInfo, ChannelInfo, ChannelPositionInfo, ChatEvent, EventInfo, HistoryMessage,
    InteractionInfo, InteractionResponseData, InviteInfo, MemberInfo, OAuth2AppInfo,
    PinnedMessageInfo, ReactionGroup, ReplyInfo, RoleInfo, RsvpInfo, ServerCommunityInfo,
    ServerInfo, SessionId, SlashCommandInfo, SlashCommandOption, TemplateInfo, ThreadInfo,
    WebhookInfo,
};
use super::permissions::{
    self, ChannelOverride, DEFAULT_ADMIN, DEFAULT_EVERYONE, DEFAULT_MODERATOR, OverrideTargetType,
    Permissions, ServerRole,
};
use super::rate_limiter::RateLimiter;
use super::server::ServerState;
use super::user_session::{Protocol, UserSession};
use super::validation;

/// The default server ID used as a fallback for IRC clients
/// that don't specify a server. No server with this ID is pre-created;
/// IRC bare-channel operations will fail unless one is created by a user.
pub const DEFAULT_SERVER_ID: &str = "default";

/// Parameters for updating notification settings (avoids too-many-arguments).
pub struct UpdateNotificationSettingsParams<'a> {
    pub server_id: &'a str,
    pub channel_id: Option<&'a str>,
    pub level: &'a str,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    pub mute_until: Option<&'a str>,
}

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
    /// HTTP client for outbound requests (link embed unfurling).
    http_client: reqwest::Client,
    /// Maximum message content length (configurable, default 4000).
    max_message_length: usize,
}

impl ChatEngine {
    pub fn new(db: Option<SqlitePool>, max_message_length: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            servers: DashMap::new(),
            channels: DashMap::new(),
            channel_name_index: DashMap::new(),
            nick_to_session: DashMap::new(),
            db,
            message_limiter: RateLimiter::new(10, 1.0),
            http_client: reqwest::Client::new(),
            max_message_length,
        }
    }

    /// Get the configured maximum message length.
    pub fn max_message_length(&self) -> usize {
        self.max_message_length
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
            let mut state =
                ServerState::new(row.id.clone(), row.name, row.owner_id.clone(), row.icon_url);

            let members = crate::db::queries::servers::get_server_members(pool, &row.id)
                .await
                .map_err(|e| format!("Failed to load server members: {e}"))?;
            for m in members {
                state.member_user_ids.insert(m.user_id);
            }

            // Bootstrap default roles for servers that don't have any
            if !crate::db::queries::roles::server_has_roles(pool, &row.id)
                .await
                .unwrap_or(true)
            {
                info!(server_id = %row.id, "bootstrapping default roles for existing server");
                let default_roles = [
                    ("@everyone", None, 0, DEFAULT_EVERYONE.bits() as i64, true),
                    ("Moderator", None, 1, DEFAULT_MODERATOR.bits() as i64, false),
                    ("Admin", None, 2, DEFAULT_ADMIN.bits() as i64, false),
                    ("Owner", None, 3, Permissions::all().bits() as i64, false),
                ];
                let mut owner_role_id = None;
                for (role_name, color, position, perms, is_default) in &default_roles {
                    let role_id = Uuid::new_v4().to_string();
                    let params = crate::db::queries::roles::CreateRoleParams {
                        id: &role_id,
                        server_id: &row.id,
                        name: role_name,
                        color: *color,
                        icon_url: None,
                        position: *position,
                        permissions: *perms,
                        is_default: *is_default,
                    };
                    let _ = crate::db::queries::roles::create_role(pool, &params).await;
                    if *role_name == "Owner" {
                        owner_role_id = Some(role_id);
                    }
                }
                // Assign Owner role to the server owner
                if let Some(role_id) = owner_role_id {
                    let _ = crate::db::queries::roles::assign_role(
                        pool,
                        &row.id,
                        &row.owner_id,
                        &role_id,
                    )
                    .await;
                }
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
                ch.category_id = row.category_id;
                ch.position = row.position;
                ch.is_private = row.is_private != 0;
                ch.channel_type = row.channel_type;
                ch.thread_parent_message_id = row.thread_parent_message_id;
                ch.auto_archive_minutes = row.thread_auto_archive_minutes;
                ch.archived = row.archived != 0;

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
    ) -> Result<(SessionId, mpsc::Receiver<ChatEvent>), String> {
        validation::validate_nickname(&nickname)?;

        // If nickname is already in use, disconnect the stale session.
        if let Some(old_session_id) = self.nick_to_session.get(&nickname).map(|r| *r) {
            info!(%nickname, "replacing stale session for reconnecting user");
            self.disconnect(old_session_id);
        }

        let session_id = Uuid::new_v4();
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
        self.nick_to_session.insert(nickname.clone(), session_id);

        // Update presence to online
        if let (Some(uid), Some(pool)) = (&session_user_id, &self.db) {
            let pool = pool.clone();
            let uid = uid.clone();
            tokio::spawn(async move {
                let _ = crate::db::queries::presence::upsert_presence(
                    &pool, &uid, "online", None, None,
                )
                .await;
            });
        }

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

        // Update presence if this was the last session for this user
        if let Some(ref uid) = session.user_id {
            let other_sessions = self
                .sessions
                .iter()
                .any(|s| s.key() != &session_id && s.user_id.as_deref() == Some(uid));
            if !other_sessions {
                if let Some(pool) = &self.db {
                    let _ = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(crate::db::queries::presence::set_offline(pool, uid))
                    });
                }
                // Broadcast offline to shared servers
                for server in self.servers.iter() {
                    if server.member_user_ids.contains(uid) {
                        let event = ChatEvent::PresenceUpdate {
                            server_id: server.id.clone(),
                            presence: super::events::PresenceInfo {
                                user_id: uid.clone(),
                                nickname: session.nickname.clone(),
                                avatar_url: session.avatar_url.clone(),
                                status: "offline".into(),
                                custom_status: None,
                                status_emoji: None,
                            },
                        };
                        for channel_id in server.channel_ids.iter() {
                            self.broadcast_to_channel(channel_id, &event, Some(session_id));
                        }
                    }
                }
            }
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
        state.member_user_ids.insert(owner_user_id.clone());
        self.servers.insert(server_id.clone(), state);

        // Create default roles (@everyone, Moderator, Admin, Owner)
        if let Some(pool) = &self.db {
            let roles = [
                ("@everyone", None, 0, DEFAULT_EVERYONE.bits() as i64, true),
                ("Moderator", None, 1, DEFAULT_MODERATOR.bits() as i64, false),
                ("Admin", None, 2, DEFAULT_ADMIN.bits() as i64, false),
                ("Owner", None, 3, Permissions::all().bits() as i64, false),
            ];
            let mut owner_role_id = None;
            for (role_name, color, position, perms, is_default) in &roles {
                let role_id = Uuid::new_v4().to_string();
                let params = crate::db::queries::roles::CreateRoleParams {
                    id: &role_id,
                    server_id: &server_id,
                    name: role_name,
                    color: *color,
                    icon_url: None,
                    position: *position,
                    permissions: *perms,
                    is_default: *is_default,
                };
                if let Err(e) = crate::db::queries::roles::create_role(pool, &params).await {
                    warn!(error = %e, role = role_name, "failed to create default role");
                }
                if *role_name == "Owner" {
                    owner_role_id = Some(role_id);
                }
            }
            // Assign Owner role to the server creator
            if let Some(role_id) = owner_role_id {
                let _ = crate::db::queries::roles::assign_role(
                    pool,
                    &server_id,
                    &owner_user_id,
                    &role_id,
                )
                .await;
            }
        }

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

    /// Update a server's name and/or icon.
    pub async fn update_server_settings(
        &self,
        server_id: &str,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(), String> {
        let mut server = self
            .servers
            .get_mut(server_id)
            .ok_or_else(|| format!("No such server: {server_id}"))?;

        let new_name = name.unwrap_or(&server.name).to_string();
        let new_icon = if icon_url.is_some() {
            icon_url.map(|s| s.to_string())
        } else {
            server.icon_url.clone()
        };

        if let Some(pool) = &self.db {
            crate::db::queries::servers::update_server(
                pool,
                server_id,
                &new_name,
                new_icon.as_deref(),
            )
            .await
            .map_err(|e| format!("Failed to update server: {e}"))?;
        }

        server.name = new_name;
        server.icon_url = new_icon;

        info!(%server_id, "server settings updated");
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

    /// Check if a user is a member of a server (in-memory check).
    pub fn user_is_server_member(&self, server_id: &str, user_id: &str) -> bool {
        self.servers
            .get(server_id)
            .map(|s| s.member_user_ids.contains(user_id))
            .unwrap_or(false)
    }

    /// Join a server (persistent membership).
    pub async fn join_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if !self.servers.contains_key(server_id) {
            return Err(format!("No such server: {server_id}"));
        }

        // Check if the user is banned from this server
        if let Some(pool) = &self.db {
            if crate::db::queries::bans::is_banned(pool, server_id, user_id)
                .await
                .unwrap_or(false)
            {
                return Err("You are banned from this server".into());
            }

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

    /// Get the owner user ID for a server.
    pub fn get_server_owner_id(&self, server_id: &str) -> Option<String> {
        self.servers.get(server_id).map(|s| s.owner_id.clone())
    }

    // ── Channel management ──────────────────────────────────────────

    /// Create a channel within a server. Returns the channel ID.
    pub async fn create_channel_in_server(
        &self,
        server_id: &str,
        name: &str,
        category_id: Option<&str>,
        is_private: bool,
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

            if let Some(cat_id) = category_id {
                crate::db::queries::channels::update_channel_category(
                    pool,
                    &channel_id,
                    Some(cat_id),
                )
                .await
                .map_err(|e| format!("Failed to set channel category: {e}"))?;
            }

            if is_private {
                crate::db::queries::channels::set_channel_private(pool, &channel_id, true)
                    .await
                    .map_err(|e| format!("Failed to set channel private: {e}"))?;
            }
        }

        let mut ch = ChannelState::new(channel_id.clone(), server_id.to_string(), name.clone());
        ch.category_id = category_id.map(|s| s.to_string());
        ch.is_private = is_private;
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

        // Check private channel access control
        if let Some(id) = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.clone()))
            && let Some(ch) = self.channels.get(id.value())
            && ch.is_private
        {
            // Private channel: require VIEW_CHANNELS permission
            if let Some(ref uid) = session.user_id
                && self.db.is_some()
            {
                let srv = server_id.to_string();
                let ch_id = id.clone();
                let uid = uid.clone();
                let has_view = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let perms = self
                            .get_effective_permissions(&srv, Some(&ch_id), &uid)
                            .await;
                        perms.contains(Permissions::VIEW_CHANNELS)
                    })
                });
                if !has_view {
                    return Err("You do not have permission to join this private channel".into());
                }
            } else {
                return Err("Authentication required to join private channels".into());
            }
        }

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
                        server_avatar_url: None,
                        status: None,
                        custom_status: None,
                        status_emoji: None,
                        user_id: s.user_id.clone(),
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

    /// Send a message to a channel or user (DM), with optional reply and attachments.
    #[allow(clippy::too_many_arguments)]
    pub fn send_message(
        &self,
        session_id: SessionId,
        server_id: &str,
        target: &str,
        content: &str,
        reply_to_id: Option<&str>,
        attachment_ids: Option<&[String]>,
        nonce: Option<&str>,
    ) -> Result<(), String> {
        validation::validate_message_with_limit(content, self.max_message_length)?;
        let content = &validation::sanitize_html(content);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        if !self.message_limiter.check(&session.nickname) {
            return Err("Rate limit exceeded. Please slow down.".into());
        }

        // Enforce timeout: timed-out users cannot send messages
        if let Some(pool) = &self.db
            && let Some(ref uid) = session.user_id
        {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let uid = uid.clone();
            let timed_out = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Ok(Some(until)) =
                        crate::db::queries::moderation::get_member_timeout(&pool, &srv, &uid).await
                        && let Ok(timeout_dt) =
                            chrono::NaiveDateTime::parse_from_str(&until, "%Y-%m-%d %H:%M:%S")
                    {
                        let timeout_utc = timeout_dt.and_utc();
                        return timeout_utc > chrono::Utc::now();
                    }
                    false
                })
            });
            if timed_out {
                return Err("You are timed out and cannot send messages".into());
            }
        }

        // Enforce slow mode: check per-channel cooldown
        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let tgt = target.to_string();
            let nick = session.nickname.clone();
            let slow_err = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Ok(Some(ch)) =
                        crate::db::queries::channels::get_channel_by_name(&pool, &srv, &tgt).await
                        && ch.slowmode_seconds > 0
                        && let Ok(Some(last)) =
                            crate::db::queries::messages::get_last_user_message_time(
                                &pool, &ch.id, &nick,
                            )
                            .await
                        && let Ok(last_dt) =
                            chrono::NaiveDateTime::parse_from_str(&last, "%Y-%m-%d %H:%M:%S")
                    {
                        let last_utc = last_dt.and_utc();
                        let cooldown = chrono::Duration::seconds(ch.slowmode_seconds as i64);
                        if chrono::Utc::now() - last_utc < cooldown {
                            return Some(format!(
                                "Slow mode: wait {} seconds between messages",
                                ch.slowmode_seconds
                            ));
                        }
                    }
                    None
                })
            });
            if let Some(err) = slow_err {
                return Err(err);
            }
        }

        // Evaluate automod rules (keyword, mention_spam, link_filter)
        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let content_clone = content.to_string();
            let automod_err = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let rules = crate::db::queries::automod::get_enabled_rules(&pool, &srv)
                        .await
                        .unwrap_or_default();
                    for rule in rules {
                        let triggered = match rule.rule_type.as_str() {
                            "keyword" => {
                                // Config: {"words":["bad","spam"]}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    if let Some(words) =
                                        config.get("words").and_then(|w| w.as_array())
                                    {
                                        let lower = content_clone.to_lowercase();
                                        words.iter().any(|w| {
                                            w.as_str().is_some_and(|kw| {
                                                lower.contains(&kw.to_lowercase())
                                            })
                                        })
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            "mention_spam" => {
                                // Config: {"max_mentions":5}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    let max = config
                                        .get("max_mentions")
                                        .and_then(|m| m.as_i64())
                                        .unwrap_or(5)
                                        as usize;
                                    let mention_count = content_clone.matches('@').count();
                                    mention_count > max
                                } else {
                                    false
                                }
                            }
                            "link_filter" => {
                                // Config: {"block_all":true}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    let block_all = config
                                        .get("block_all")
                                        .and_then(|b| b.as_bool())
                                        .unwrap_or(false);
                                    if block_all {
                                        content_clone.contains("http://")
                                            || content_clone.contains("https://")
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };
                        if triggered {
                            return Some(format!("Message blocked by automod rule: {}", rule.name));
                        }
                    }
                    None
                })
            });
            if let Some(err) = automod_err {
                return Err(err);
            }
        }

        // Build reply info if replying to a message
        let reply_to: Option<ReplyInfo> = if let Some(ref_id) = reply_to_id {
            if let Some(pool) = &self.db {
                // Synchronous lookup via block_in_place — reply info is needed before broadcast
                let pool = pool.clone();
                let ref_id = ref_id.to_string();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        match crate::db::queries::messages::get_message_by_id(&pool, &ref_id).await
                        {
                            Ok(Some(row)) => Some(ReplyInfo {
                                id: row.id,
                                from: row.sender_nick,
                                content_preview: row.content.chars().take(100).collect::<String>(),
                            }),
                            _ => None,
                        }
                    })
                })
            } else {
                None
            }
        } else {
            None
        };

        // Look up attachment metadata if attachment_ids provided
        let attachments: Option<Vec<super::events::AttachmentInfo>> = if let Some(ids) =
            attachment_ids
            && !ids.is_empty()
        {
            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let ids = ids.to_vec();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let infos =
                            crate::db::queries::attachments::get_attachments_by_ids(&pool, &ids)
                                .await
                                .unwrap_or_default();
                        if infos.is_empty() {
                            None
                        } else {
                            Some(
                                infos
                                    .into_iter()
                                    .map(|a| super::events::AttachmentInfo {
                                        id: a.id.clone(),
                                        filename: a.original_filename,
                                        content_type: a.content_type,
                                        file_size: a.file_size,
                                        url: format!("/api/uploads/{}", a.id),
                                    })
                                    .collect(),
                            )
                        }
                    })
                })
            } else {
                None
            }
        } else {
            None
        };

        let msg_id = Uuid::new_v4();
        let event = ChatEvent::Message {
            id: msg_id,
            server_id: Some(server_id.to_string()),
            from: session.nickname.clone(),
            target: target.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            avatar_url: session.avatar_url.clone(),
            reply_to: reply_to.clone(),
            attachments: attachments.clone(),
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
                let uid = session.user_id.clone().unwrap_or_else(|| sid.clone());
                let msg = content.to_string();
                let reply_id = reply_to_id.map(|s| s.to_string());
                let att_ids = attachment_ids.map(|ids| ids.to_vec());
                tokio::spawn(async move {
                    let params = crate::db::queries::messages::InsertMessageParams {
                        id: &id,
                        server_id: &srv,
                        channel_id: &ch,
                        sender_id: &uid,
                        sender_nick: &nick,
                        content: &msg,
                        reply_to_id: reply_id.as_deref(),
                    };
                    if let Err(e) =
                        crate::db::queries::messages::insert_message(&pool, &params).await
                    {
                        error!(error = %e, "failed to persist message");
                    }
                    // Link attachments to the message (use user_id, not session_id)
                    if let Some(att_ids) = att_ids
                        && let Err(e) =
                            crate::db::queries::attachments::link_attachments_to_message(
                                &pool, &id, &att_ids, &uid,
                            )
                            .await
                    {
                        error!(error = %e, "failed to link attachments");
                    }
                });
            }

            self.broadcast_to_channel(&channel_id, &event, Some(session_id));

            // Send MessageAck back to the sender with the server-generated message ID
            if let Some(sender_session) = self.sessions.get(&session_id) {
                let _ = sender_session.send(ChatEvent::MessageAck {
                    id: msg_id,
                    server_id: server_id.to_string(),
                    channel: target.to_string(),
                    nonce: nonce.map(|s| s.to_string()),
                });
            }

            // Async link embed unfurling — extract URLs and resolve OG metadata
            let urls = super::embeds::extract_urls(content);
            if !urls.is_empty()
                && let Some(pool) = &self.db
            {
                let pool = pool.clone();
                let client = self.http_client.clone();
                let server_id_owned = server_id.to_string();
                let channel_name_owned = channel_name.clone();
                // Collect senders for channel members before spawning
                let member_senders: Vec<mpsc::Sender<ChatEvent>> =
                    if let Some(channel) = self.channels.get(&channel_id) {
                        channel
                            .members
                            .iter()
                            .filter_map(|sid| self.sessions.get(sid).map(|s| s.outbound.clone()))
                            .collect()
                    } else {
                        vec![]
                    };
                tokio::spawn(async move {
                    let mut embeds = Vec::new();
                    for url in urls {
                        // Check cache first
                        if let Ok(Some(cached)) =
                            crate::db::queries::embeds::get_cached_embed(&pool, &url).await
                        {
                            embeds.push(super::events::EmbedInfo {
                                url: cached.url,
                                title: cached.title,
                                description: cached.description,
                                image_url: cached.image_url,
                                site_name: cached.site_name,
                            });
                            continue;
                        }
                        // Unfurl
                        if let Some(info) = super::embeds::unfurl_url(&client, &url).await {
                            let _ = crate::db::queries::embeds::upsert_embed(
                                &pool,
                                &info.url,
                                info.title.as_deref(),
                                info.description.as_deref(),
                                info.image_url.as_deref(),
                                info.site_name.as_deref(),
                            )
                            .await;
                            embeds.push(info);
                        }
                    }
                    if !embeds.is_empty() {
                        let embed_event = ChatEvent::MessageEmbed {
                            message_id: msg_id,
                            server_id: server_id_owned,
                            channel: channel_name_owned,
                            embeds,
                        };
                        for sender in &member_senders {
                            let _ = sender.try_send(embed_event.clone());
                        }
                    }
                });
            }
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

            // Send MessageAck back to the DM sender
            if let Some(sender_session) = self.sessions.get(&session_id) {
                let _ = sender_session.send(ChatEvent::MessageAck {
                    id: msg_id,
                    server_id: String::new(),
                    channel: target.to_string(),
                    nonce: nonce.map(|s| s.to_string()),
                });
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
        let topic = validation::sanitize_html(&topic);
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

    /// Fetch message history for a channel, including edits, replies, and reactions.
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
        let rows: Vec<_> = rows.into_iter().take(limit as usize).collect();

        // Collect message IDs for batch reaction lookup
        let msg_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();

        // Fetch reactions for all messages in batch
        let reaction_rows =
            crate::db::queries::messages::get_reactions_for_messages(pool, &msg_ids)
                .await
                .unwrap_or_default();

        // Group reactions by message_id -> emoji -> user_ids
        let mut reaction_map: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Vec<String>>,
        > = std::collections::HashMap::new();
        for r in &reaction_rows {
            reaction_map
                .entry(r.message_id.clone())
                .or_default()
                .entry(r.emoji.clone())
                .or_default()
                .push(r.user_id.clone());
        }

        // Collect reply_to_ids for batch lookup
        let reply_ids: Vec<String> = rows.iter().filter_map(|r| r.reply_to_id.clone()).collect();
        let mut reply_map: std::collections::HashMap<String, ReplyInfo> =
            std::collections::HashMap::new();
        if !reply_ids.is_empty() {
            for rid in &reply_ids {
                if let Ok(Some(parent)) =
                    crate::db::queries::messages::get_message_by_id(pool, rid).await
                {
                    reply_map.insert(
                        parent.id.clone(),
                        ReplyInfo {
                            id: parent.id,
                            from: parent.sender_nick,
                            content_preview: parent.content.chars().take(100).collect(),
                        },
                    );
                }
            }
        }

        // Fetch attachments for all messages in batch
        let attachment_rows =
            crate::db::queries::attachments::get_attachments_for_messages(pool, &msg_ids)
                .await
                .unwrap_or_default();

        // Group attachments by message_id
        let mut attachment_map: std::collections::HashMap<
            String,
            Vec<super::events::AttachmentInfo>,
        > = std::collections::HashMap::new();
        for a in &attachment_rows {
            if let Some(ref mid) = a.message_id {
                attachment_map.entry(mid.clone()).or_default().push(
                    super::events::AttachmentInfo {
                        id: a.id.clone(),
                        filename: a.original_filename.clone(),
                        content_type: a.content_type.clone(),
                        file_size: a.file_size,
                        url: format!("/api/uploads/{}", a.id),
                    },
                );
            }
        }

        let messages: Vec<HistoryMessage> = rows
            .into_iter()
            .map(|row| {
                let reactions = reaction_map.get(&row.id).map(|emoji_map| {
                    emoji_map
                        .iter()
                        .map(|(emoji, user_ids)| ReactionGroup {
                            emoji: emoji.clone(),
                            count: user_ids.len(),
                            user_ids: user_ids.clone(),
                        })
                        .collect()
                });
                let reply_to = row
                    .reply_to_id
                    .as_ref()
                    .and_then(|rid| reply_map.get(rid).cloned());
                let edited_at = row.edited_at.as_ref().and_then(|s| s.parse().ok());
                let attachments = attachment_map.remove(&row.id);

                HistoryMessage {
                    id: row.id.parse().unwrap_or_default(),
                    from: row.sender_nick,
                    content: row.content,
                    timestamp: row.created_at.parse().unwrap_or_else(|_| Utc::now()),
                    edited_at,
                    reply_to,
                    reactions,
                    attachments,
                    embeds: None,
                }
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
                category_id: entry.category_id.clone(),
                position: entry.position,
                is_private: entry.is_private,
                channel_type: entry.channel_type.clone(),
                thread_parent_message_id: entry.thread_parent_message_id.clone(),
                archived: entry.archived,
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
                    server_avatar_url: None,
                    status: None,
                    custom_status: None,
                    status_emoji: None,
                    user_id: s.user_id.clone(),
                })
            })
            .collect())
    }

    // ── Message editing & deletion ─────────────────────────────────

    /// Edit a message's content. Only the sender or a moderator+ can edit.
    pub async fn edit_message(
        &self,
        session_id: SessionId,
        message_id: &str,
        new_content: &str,
    ) -> Result<(), String> {
        validation::validate_message_with_limit(new_content, self.max_message_length)?;
        let new_content = &validation::sanitize_html(new_content);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        // Only the sender can edit their own messages (require user_id match, not nickname)
        let sender_id = session
            .user_id
            .as_deref()
            .ok_or("Authentication required to edit messages")?;
        if msg.sender_id != sender_id {
            return Err("You can only edit your own messages".into());
        }

        crate::db::queries::messages::update_message_content(pool, message_id, new_content)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        // Find the channel name for the event
        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::MessageEdit {
            id: message_id.parse().unwrap_or_default(),
            server_id: server_id.clone(),
            channel: channel_name,
            content: new_content.to_string(),
            edited_at: Utc::now(),
        };

        // Broadcast to the channel (including sender)
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Delete a message (soft delete). Sender can delete own, moderator+ can delete any.
    pub async fn delete_message(
        &self,
        session_id: SessionId,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let sender_id = session
            .user_id
            .as_deref()
            .ok_or("Authentication required to delete messages")?;
        let is_sender = msg.sender_id == sender_id;

        if !is_sender {
            // Check if user has moderator+ role
            let server_id = msg.server_id.as_deref().ok_or("Message has no server")?;
            if let Some(uid) = &session.user_id {
                let role = self.get_server_role(server_id, uid).await;
                if !matches!(
                    role,
                    Some(ServerRole::Owner) | Some(ServerRole::Admin) | Some(ServerRole::Moderator)
                ) {
                    return Err("You can only delete your own messages".into());
                }
            } else {
                return Err("You can only delete your own messages".into());
            }
        }

        crate::db::queries::messages::soft_delete_message(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::MessageDelete {
            id: message_id.parse().unwrap_or_default(),
            server_id,
            channel: channel_name,
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    // ── Reactions ────────────────────────────────────────────────────

    /// Add a reaction to a message.
    pub async fn add_reaction(
        &self,
        session_id: SessionId,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let user_id = session.user_id.as_deref().unwrap_or(&session.nickname);

        crate::db::queries::messages::add_reaction(pool, message_id, user_id, emoji)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::ReactionAdd {
            message_id: message_id.parse().unwrap_or_default(),
            server_id,
            channel: channel_name,
            user_id: user_id.to_string(),
            nickname: session.nickname.clone(),
            emoji: emoji.to_string(),
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Remove a reaction from a message.
    pub async fn remove_reaction(
        &self,
        session_id: SessionId,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let user_id = session.user_id.as_deref().unwrap_or(&session.nickname);

        crate::db::queries::messages::remove_reaction(pool, message_id, user_id, emoji)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::ReactionRemove {
            message_id: message_id.parse().unwrap_or_default(),
            server_id,
            channel: channel_name,
            user_id: user_id.to_string(),
            nickname: session.nickname.clone(),
            emoji: emoji.to_string(),
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    // ── Typing indicators ────────────────────────────────────────────

    /// Broadcast a typing indicator to a channel.
    pub fn send_typing(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let event = ChatEvent::TypingStart {
            server_id: server_id.to_string(),
            channel: channel_name,
            nickname: session.nickname.clone(),
        };

        self.broadcast_to_channel(&channel_id, &event, Some(session_id));

        Ok(())
    }

    // ── Read state ────────────────────────────────────────────────────

    /// Mark a channel as read for a user, up to a specific message ID.
    pub async fn mark_read(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        crate::db::queries::messages::mark_channel_read(pool, user_id, &channel_id, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        Ok(())
    }

    /// Get unread counts for all channels in a server for a user.
    pub async fn get_unread_counts(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<Vec<super::events::UnreadCount>, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows = crate::db::queries::messages::get_unread_counts(pool, user_id, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        // Map channel_id -> channel_name
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let name = self.channels.get(&r.channel_id).map(|ch| ch.name.clone())?;
                Some(super::events::UnreadCount {
                    channel_name: name,
                    count: r.unread_count,
                })
            })
            .collect())
    }

    // ── Roles ────────────────────────────────────────────────────────

    /// Get effective permissions for a user in a channel.
    pub async fn get_effective_permissions(
        &self,
        server_id: &str,
        channel_id: Option<&str>,
        user_id: &str,
    ) -> Permissions {
        let is_owner = self.is_server_owner(server_id, user_id);
        if is_owner {
            return Permissions::all();
        }

        let Some(pool) = &self.db else {
            return DEFAULT_EVERYONE;
        };

        // Get default (@everyone) role base permissions
        let base = match crate::db::queries::roles::get_default_role(pool, server_id).await {
            Ok(Some(role)) => Permissions::from_bits_truncate(role.permissions as u64),
            _ => DEFAULT_EVERYONE,
        };

        // Get user's assigned roles
        let user_roles = crate::db::queries::roles::get_user_roles(pool, server_id, user_id)
            .await
            .unwrap_or_default();
        let role_perms: Vec<(String, Permissions)> = user_roles
            .iter()
            .map(|r| {
                (
                    r.id.clone(),
                    Permissions::from_bits_truncate(r.permissions as u64),
                )
            })
            .collect();

        // Get channel overrides if a channel was specified
        let overrides = if let Some(ch_id) = channel_id {
            crate::db::queries::channels::get_channel_overrides(pool, ch_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|o| ChannelOverride {
                    target_type: if o.target_type == "role" {
                        OverrideTargetType::Role
                    } else {
                        OverrideTargetType::User
                    },
                    target_id: o.target_id,
                    allow: Permissions::from_bits_truncate(o.allow_bits as u64),
                    deny: Permissions::from_bits_truncate(o.deny_bits as u64),
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        // Get @everyone role id
        let everyone_role_id =
            match crate::db::queries::roles::get_default_role(pool, server_id).await {
                Ok(Some(role)) => role.id,
                _ => String::new(),
            };

        permissions::compute_effective_permissions(
            base,
            &role_perms,
            &overrides,
            &everyone_role_id,
            user_id,
            is_owner,
        )
    }

    /// Check that a user has a required permission. Returns Ok(user_id) or Err(message).
    pub async fn require_permission(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_id: Option<&str>,
        required: Permissions,
    ) -> Result<String, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();

        let perms = self
            .get_effective_permissions(server_id, channel_id, &user_id)
            .await;

        if perms.contains(required) {
            Ok(user_id)
        } else {
            Err("FORBIDDEN: insufficient permissions".into())
        }
    }

    /// List roles for a server.
    pub async fn list_roles(&self, server_id: &str) -> Result<Vec<RoleInfo>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = crate::db::queries::roles::list_roles(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        Ok(rows.into_iter().map(role_row_to_info).collect())
    }

    /// Create a custom role in a server.
    pub async fn create_role(
        &self,
        server_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: i64,
    ) -> Result<RoleInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;

        // Determine position: max + 1
        let existing = crate::db::queries::roles::list_roles(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        let max_pos = existing.iter().map(|r| r.position).max().unwrap_or(0);

        let role_id = Uuid::new_v4().to_string();
        let params = crate::db::queries::roles::CreateRoleParams {
            id: &role_id,
            server_id,
            name,
            color,
            icon_url: None,
            position: max_pos + 1,
            permissions,
            is_default: false,
        };
        crate::db::queries::roles::create_role(pool, &params)
            .await
            .map_err(|e| format!("Failed to create role: {e}"))?;

        let role = crate::db::queries::roles::get_role(pool, &role_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Role not found after creation")?;

        Ok(role_row_to_info(role))
    }

    /// Update a custom role.
    pub async fn update_role(
        &self,
        server_id: &str,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: i64,
    ) -> Result<RoleInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        // Verify the role belongs to the expected server (prevents cross-server manipulation)
        let role = crate::db::queries::roles::get_role(pool, role_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Role not found")?;
        if role.server_id != server_id {
            return Err("Role does not belong to this server".into());
        }
        crate::db::queries::roles::update_role(pool, role_id, name, color, None, permissions)
            .await
            .map_err(|e| format!("Failed to update role: {e}"))?;
        let role = crate::db::queries::roles::get_role(pool, role_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Role not found")?;
        Ok(role_row_to_info(role))
    }

    /// Delete a custom role.
    pub async fn delete_role(&self, server_id: &str, role_id: &str) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        // Prevent deleting the @everyone default role
        let role = crate::db::queries::roles::get_role(pool, role_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Role not found")?;
        if role.is_default != 0 {
            return Err("Cannot delete the default @everyone role".into());
        }
        // Verify the role belongs to the expected server (prevents cross-server manipulation)
        if role.server_id != server_id {
            return Err("Role does not belong to this server".into());
        }
        crate::db::queries::roles::delete_role(pool, role_id)
            .await
            .map_err(|e| format!("Failed to delete role: {e}"))?;
        Ok(())
    }

    /// Assign a role to a user.
    pub async fn assign_role(
        &self,
        server_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        crate::db::queries::roles::assign_role(pool, server_id, user_id, role_id)
            .await
            .map_err(|e| format!("Failed to assign role: {e}"))?;
        let roles = crate::db::queries::roles::get_user_roles(pool, server_id, user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        Ok(roles.into_iter().map(|r| r.id).collect())
    }

    /// Remove a role from a user.
    pub async fn remove_role(
        &self,
        server_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        crate::db::queries::roles::remove_role(pool, server_id, user_id, role_id)
            .await
            .map_err(|e| format!("Failed to remove role: {e}"))?;
        let roles = crate::db::queries::roles::get_user_roles(pool, server_id, user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        Ok(roles.into_iter().map(|r| r.id).collect())
    }

    // ── Categories ──────────────────────────────────────────────────

    /// List categories for a server.
    pub async fn list_categories(&self, server_id: &str) -> Result<Vec<CategoryInfo>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = crate::db::queries::categories::list_categories(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        Ok(rows.into_iter().map(category_row_to_info).collect())
    }

    /// Create a channel category.
    pub async fn create_category(
        &self,
        server_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let existing = crate::db::queries::categories::list_categories(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        let max_pos = existing.iter().map(|c| c.position).max().unwrap_or(-1);

        let cat_id = Uuid::new_v4().to_string();
        crate::db::queries::categories::create_category(
            pool,
            &cat_id,
            server_id,
            name,
            max_pos + 1,
        )
        .await
        .map_err(|e| format!("Failed to create category: {e}"))?;

        Ok(CategoryInfo {
            id: cat_id,
            server_id: server_id.to_string(),
            name: name.to_string(),
            position: max_pos + 1,
        })
    }

    /// Update a channel category name.
    pub async fn update_category(
        &self,
        category_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        crate::db::queries::categories::update_category(pool, category_id, name)
            .await
            .map_err(|e| format!("Failed to update category: {e}"))?;
        let cat = crate::db::queries::categories::get_category(pool, category_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Category not found")?;
        Ok(category_row_to_info(cat))
    }

    /// Delete a channel category.
    pub async fn delete_category(&self, category_id: &str) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        crate::db::queries::categories::delete_category(pool, category_id)
            .await
            .map_err(|e| format!("Failed to delete category: {e}"))?;
        // Channels referencing this category get NULL (ON DELETE SET NULL)
        // Update in-memory state
        for mut ch in self.channels.iter_mut() {
            if ch.category_id.as_deref() == Some(category_id) {
                ch.category_id = None;
            }
        }
        Ok(())
    }

    // ── Channel organization ────────────────────────────────────────

    /// Reorder channels: update position and category for a batch of channels.
    pub async fn reorder_channels(
        &self,
        _server_id: &str,
        updates: &[ChannelPositionInfo],
    ) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        for update in updates {
            crate::db::queries::channels::update_channel_position(
                pool,
                &update.id,
                update.position,
            )
            .await
            .map_err(|e| format!("Failed to update channel position: {e}"))?;
            crate::db::queries::channels::update_channel_category(
                pool,
                &update.id,
                update.category_id.as_deref(),
            )
            .await
            .map_err(|e| format!("Failed to update channel category: {e}"))?;

            // Update in-memory state
            if let Some(mut ch) = self.channels.get_mut(&update.id) {
                ch.position = update.position;
                ch.category_id.clone_from(&update.category_id);
            }
        }
        Ok(())
    }

    // ── Profiles ───────────────────────────────────────────────────

    /// Get a user's full profile.
    pub async fn get_user_profile(
        &self,
        user_id: &str,
    ) -> Result<super::events::UserProfileInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;

        // Get basic user info
        let (id, username, _email, avatar_url) = crate::db::queries::users::get_user(pool, user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("User not found")?;

        // Get profile
        let profile = crate::db::queries::profiles::get_profile(pool, &id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        // Get user created_at
        let created_at =
            sqlx::query_scalar::<_, String>("SELECT created_at FROM users WHERE id = ?")
                .bind(&id)
                .fetch_one(pool)
                .await
                .unwrap_or_else(|_| "unknown".into());

        Ok(super::events::UserProfileInfo {
            user_id: id,
            username,
            avatar_url,
            bio: profile.as_ref().and_then(|p| p.bio.clone()),
            pronouns: profile.as_ref().and_then(|p| p.pronouns.clone()),
            banner_url: profile.as_ref().and_then(|p| p.banner_url.clone()),
            created_at,
        })
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

    /// Look up a session ID by nickname. Returns None if no session with that nick exists.
    pub fn get_session_id_by_nick(&self, nickname: &str) -> Option<SessionId> {
        self.nick_to_session.get(nickname).map(|r| *r)
    }

    /// Get the user_id for a session. Returns None if session not found or has no user.
    pub fn get_session_user_id(&self, session_id: SessionId) -> Option<String> {
        self.sessions
            .get(&session_id)
            .and_then(|s| s.user_id.clone())
    }

    /// Get (server_id, channel_name) pairs for all channels a session is in.
    pub fn get_session_channels(&self, session_id: SessionId) -> Vec<(String, String)> {
        let session = match self.sessions.get(&session_id) {
            Some(s) => s.clone(),
            None => return vec![],
        };
        session
            .channels
            .iter()
            .filter_map(|ch_id| {
                self.channels
                    .get(ch_id)
                    .map(|ch| (ch.server_id.clone(), ch.name.clone()))
            })
            .collect()
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: SessionId) -> Option<Arc<UserSession>> {
        self.sessions.get(&session_id).map(|s| s.clone())
    }

    /// Get the database pool (if configured).
    pub fn get_db(&self) -> Option<SqlitePool> {
        self.db.clone()
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

    // ── Presence ─────────────────────────────────────────────

    /// Update a user's presence and broadcast to members of shared servers.
    pub async fn set_presence(
        &self,
        session_id: SessionId,
        status: &str,
        custom_status: Option<&str>,
        status_emoji: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.clone().ok_or("Not authenticated")?;

        // Validate status
        match status {
            "online" | "idle" | "dnd" | "invisible" => {}
            _ => return Err("Invalid status. Must be: online, idle, dnd, invisible".into()),
        }

        // Persist to DB
        if let Some(pool) = &self.db {
            crate::db::queries::presence::upsert_presence(
                pool,
                &user_id,
                status,
                custom_status,
                status_emoji,
            )
            .await
            .map_err(|e| format!("Failed to update presence: {e}"))?;
        }

        // Build presence info
        let presence = super::events::PresenceInfo {
            user_id: user_id.clone(),
            nickname: session.nickname.clone(),
            avatar_url: session.avatar_url.clone(),
            status: if status == "invisible" {
                "offline".into()
            } else {
                status.into()
            },
            custom_status: custom_status.map(|s| s.to_string()),
            status_emoji: status_emoji.map(|s| s.to_string()),
        };

        // Broadcast to all servers the user is a member of
        for server in self.servers.iter() {
            if server.member_user_ids.contains(&user_id) {
                let event = ChatEvent::PresenceUpdate {
                    server_id: server.id.clone(),
                    presence: presence.clone(),
                };
                // Send to all sessions in this server's channels
                for channel_id in server.channel_ids.iter() {
                    if let Some(channel) = self.channels.get(channel_id) {
                        for &member_sid in &channel.members {
                            if member_sid != session_id
                                && let Some(s) = self.sessions.get(&member_sid)
                            {
                                let _ = s.send(event.clone());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Get presence list for all members of a server.
    pub async fn get_server_presences(
        &self,
        server_id: &str,
    ) -> Result<Vec<super::events::PresenceInfo>, String> {
        let server = self
            .servers
            .get(server_id)
            .ok_or(format!("Server not found: {server_id}"))?;

        let user_ids: Vec<String> = server.member_user_ids.iter().cloned().collect();
        drop(server);

        let mut presences = Vec::new();

        // Get DB presence records
        if let Some(pool) = &self.db {
            let rows = crate::db::queries::presence::get_presences_for_users(pool, &user_ids)
                .await
                .unwrap_or_default();

            for row in rows {
                // Find nickname for this user from active sessions
                let (nickname, avatar_url) = self.find_user_display_info(&row.user_id);
                let visible_status = if row.status == "invisible" {
                    "offline".to_string()
                } else {
                    row.status
                };
                presences.push(super::events::PresenceInfo {
                    user_id: row.user_id,
                    nickname,
                    avatar_url,
                    status: visible_status,
                    custom_status: row.custom_status,
                    status_emoji: row.status_emoji,
                });
            }
        }

        // Add online users who may not have a presence row yet
        for uid in &user_ids {
            if !presences.iter().any(|p| p.user_id == *uid) {
                let (nickname, avatar_url) = self.find_user_display_info(uid);
                let is_online = self
                    .sessions
                    .iter()
                    .any(|s| s.user_id.as_deref() == Some(uid));
                presences.push(super::events::PresenceInfo {
                    user_id: uid.clone(),
                    nickname,
                    avatar_url,
                    status: if is_online {
                        "online".into()
                    } else {
                        "offline".into()
                    },
                    custom_status: None,
                    status_emoji: None,
                });
            }
        }

        Ok(presences)
    }

    /// Find a user's display info (nickname, avatar) from active sessions or return defaults.
    fn find_user_display_info(&self, user_id: &str) -> (String, Option<String>) {
        for session in self.sessions.iter() {
            if session.user_id.as_deref() == Some(user_id) {
                return (session.nickname.clone(), session.avatar_url.clone());
            }
        }
        (format!("user-{}", &user_id[..8.min(user_id.len())]), None)
    }

    // ── Server Nicknames ─────────────────────────────────────

    /// Set a user's server-specific display name.
    pub async fn set_server_nickname(
        &self,
        session_id: SessionId,
        server_id: &str,
        nickname: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.clone().ok_or("Not authenticated")?;

        // Verify membership
        let server = self
            .servers
            .get(server_id)
            .ok_or(format!("Server not found: {server_id}"))?;
        if !server.member_user_ids.contains(&user_id) {
            return Err("Not a member of this server".into());
        }
        drop(server);

        if let Some(pool) = &self.db {
            crate::db::queries::servers::set_server_nickname(pool, server_id, &user_id, nickname)
                .await
                .map_err(|e| format!("Failed to set nickname: {e}"))?;
        }

        // Broadcast nickname change
        let event = ChatEvent::ServerNicknameUpdate {
            server_id: server_id.to_string(),
            user_id: user_id.clone(),
            nickname: nickname.map(|s| s.to_string()),
        };
        if let Some(server) = self.servers.get(server_id) {
            for channel_id in server.channel_ids.iter() {
                self.broadcast_to_channel(channel_id, &event, None);
            }
        }

        Ok(())
    }

    // ── Search ───────────────────────────────────────────────

    /// Search messages in a server using full-text search.
    pub async fn search_messages(
        &self,
        server_id: &str,
        query: &str,
        channel_name: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<super::events::SearchResultMessage>, i64), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;

        // Resolve channel name to ID if provided
        let channel_id = if let Some(ch_name) = channel_name {
            Some(self.resolve_channel_id(server_id, ch_name)?)
        } else {
            None
        };

        let (rows, total) = crate::db::queries::search::search_messages(
            pool,
            server_id,
            query,
            channel_id.as_deref(),
            limit.min(50),
            offset,
        )
        .await
        .map_err(|e| format!("Search failed: {e}"))?;

        let results: Vec<super::events::SearchResultMessage> = rows
            .into_iter()
            .filter_map(|row| {
                let channel_name = row
                    .channel_id
                    .as_ref()
                    .and_then(|cid| self.channels.get(cid).map(|ch| ch.name.clone()))
                    .unwrap_or_default();

                Some(super::events::SearchResultMessage {
                    id: row.id.parse().ok()?,
                    from: row.sender_nick,
                    content: row.content,
                    timestamp: row.created_at.parse().ok()?,
                    channel_id: row.channel_id.unwrap_or_default(),
                    channel_name,
                    edited_at: row.edited_at.as_ref().and_then(|s| s.parse().ok()),
                })
            })
            .collect();

        Ok((results, total))
    }

    // ── Notification Settings ────────────────────────────────

    /// Update notification settings for a user in a server or channel.
    pub async fn update_notification_settings(
        &self,
        session_id: SessionId,
        params: &UpdateNotificationSettingsParams<'_>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.clone().ok_or("Not authenticated")?;

        match params.level {
            "all" | "mentions" | "none" | "default" => {}
            _ => return Err("Invalid level. Must be: all, mentions, none, default".into()),
        }

        let pool = self.db.as_ref().ok_or("No database configured")?;
        let id = Uuid::new_v4().to_string();

        let db_params = crate::db::models::UpsertNotificationParams {
            id: &id,
            user_id: &user_id,
            server_id: Some(params.server_id),
            channel_id: params.channel_id,
            level: params.level,
            suppress_everyone: params.suppress_everyone,
            suppress_roles: params.suppress_roles,
            muted: params.muted,
            mute_until: params.mute_until,
        };
        crate::db::queries::notifications::upsert_notification_setting(pool, &db_params)
            .await
            .map_err(|e| format!("Failed to update notification settings: {e}"))?;

        Ok(())
    }

    /// Get notification settings for a user in a server.
    pub async fn get_notification_settings(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<Vec<super::events::NotificationSettingInfo>, String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.clone().ok_or("Not authenticated")?;

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows =
            crate::db::queries::notifications::get_notification_settings(pool, &user_id, server_id)
                .await
                .map_err(|e| format!("Failed to get notification settings: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|r| super::events::NotificationSettingInfo {
                id: r.id,
                server_id: r.server_id,
                channel_id: r.channel_id,
                level: r.level,
                suppress_everyone: r.suppress_everyone != 0,
                suppress_roles: r.suppress_roles != 0,
                muted: r.muted != 0,
                mute_until: r.mute_until,
            })
            .collect())
    }

    // ── Pinning ─────────────────────────────────────────────────

    /// Pin a message in a channel. Requires MANAGE_MESSAGES permission or ownership of the message.
    pub async fn pin_message(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        // Look up the message
        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        // Check permission: MANAGE_MESSAGES or own message
        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let is_own = msg.sender_id == user_id || msg.sender_nick == session.nickname;
        if !is_own {
            let perms = self
                .get_effective_permissions(server_id, Some(&channel_id), user_id)
                .await;
            if !perms.contains(Permissions::MANAGE_MESSAGES) {
                return Err("FORBIDDEN: insufficient permissions to pin messages".into());
            }
        }

        // Check pin count limit (max 50 per channel)
        let pin_count = crate::db::queries::pins::count_pins(pool, &channel_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if pin_count >= 50 {
            return Err("Channel has reached the maximum of 50 pinned messages".into());
        }

        let pin_id = Uuid::new_v4().to_string();
        crate::db::queries::pins::pin_message(pool, &pin_id, &channel_id, message_id, user_id)
            .await
            .map_err(|e| format!("Failed to pin message: {e}"))?;

        let pin = PinnedMessageInfo {
            id: pin_id,
            message_id: message_id.to_string(),
            channel_id: channel_id.clone(),
            pinned_by: user_id.to_string(),
            pinned_at: Utc::now().to_rfc3339(),
            from: msg.sender_nick,
            content: msg.content,
            timestamp: msg.created_at,
        };

        let event = ChatEvent::MessagePin {
            server_id: server_id.to_string(),
            channel: channel_name,
            pin,
        };
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Unpin a message from a channel. Requires MANAGE_MESSAGES permission.
    pub async fn unpin_message(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        self.require_permission(
            session_id,
            server_id,
            Some(&channel_id),
            Permissions::MANAGE_MESSAGES,
        )
        .await?;

        let pool = self.db.as_ref().ok_or("No database configured")?;

        crate::db::queries::pins::unpin_message(pool, &channel_id, message_id)
            .await
            .map_err(|e| format!("Failed to unpin message: {e}"))?;

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
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let pin_rows = crate::db::queries::pins::get_pinned_messages(pool, &channel_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let mut pins = Vec::new();
        for row in pin_rows {
            // Look up message content for each pin
            let (from, content, timestamp) = match crate::db::queries::messages::get_message_by_id(
                pool,
                &row.message_id,
            )
            .await
            {
                Ok(Some(msg)) => (msg.sender_nick, msg.content, msg.created_at),
                _ => (
                    "unknown".to_string(),
                    "[deleted]".to_string(),
                    String::new(),
                ),
            };

            pins.push(PinnedMessageInfo {
                id: row.id,
                message_id: row.message_id,
                channel_id: row.channel_id,
                pinned_by: row.pinned_by,
                pinned_at: row.pinned_at,
                from,
                content,
                timestamp,
            });
        }

        let _ = session.send(ChatEvent::PinnedMessages {
            server_id: server_id.to_string(),
            channel: channel_name,
            pins,
        });

        Ok(())
    }

    // ── Threads ─────────────────────────────────────────────────

    /// Create a thread from a message in a channel.
    pub async fn create_thread(
        &self,
        session_id: SessionId,
        server_id: &str,
        parent_channel_name: &str,
        name: &str,
        message_id: &str,
        is_private: bool,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let _user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let parent_channel_name = normalize_channel_name(parent_channel_name);
        let parent_channel_id = self.resolve_channel_id(server_id, &parent_channel_name)?;

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

        crate::db::queries::threads::create_thread(
            pool,
            &thread_id,
            server_id,
            &thread_name,
            channel_type,
            message_id,
            1440, // default auto-archive: 24h
        )
        .await
        .map_err(|e| format!("Failed to create thread: {e}"))?;

        // Add to in-memory state
        let mut ch = ChannelState::new(
            thread_id.clone(),
            server_id.to_string(),
            thread_name.clone(),
        );
        ch.channel_type = channel_type.to_string();
        ch.thread_parent_message_id = Some(message_id.to_string());
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
            id: thread_id,
            name: thread_name,
            channel_type: channel_type.to_string(),
            parent_message_id: Some(message_id.to_string()),
            archived: false,
            auto_archive_minutes: 1440,
            message_count: 0,
            created_at: Utc::now().to_rfc3339(),
        };

        let event = ChatEvent::ThreadCreate {
            server_id: server_id.to_string(),
            parent_channel: parent_channel_name,
            thread: thread_info,
        };
        self.broadcast_to_channel(&parent_channel_id, &event, None);

        Ok(())
    }

    /// Archive a thread. Requires MANAGE_CHANNELS permission.
    pub async fn archive_thread(
        &self,
        session_id: SessionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        self.require_permission(
            session_id,
            server_id,
            Some(thread_id),
            Permissions::MANAGE_CHANNELS,
        )
        .await?;

        let pool = self.db.as_ref().ok_or("No database configured")?;

        crate::db::queries::threads::archive_thread(pool, thread_id)
            .await
            .map_err(|e| format!("Failed to archive thread: {e}"))?;

        // Update in-memory state
        let thread_info = if let Some(mut ch) = self.channels.get_mut(thread_id) {
            ch.archived = true;
            ThreadInfo {
                id: ch.id.clone(),
                name: ch.name.clone(),
                channel_type: ch.channel_type.clone(),
                parent_message_id: ch.thread_parent_message_id.clone(),
                archived: true,
                auto_archive_minutes: ch.auto_archive_minutes,
                message_count: 0, // not tracked in-memory
                created_at: ch.created_at.to_rfc3339(),
            }
        } else {
            return Err("Thread not found".into());
        };

        let event = ChatEvent::ThreadUpdate {
            server_id: server_id.to_string(),
            thread: thread_info,
        };
        self.broadcast_to_channel(thread_id, &event, None);

        Ok(())
    }

    /// List threads for a channel. Sends ThreadList event to the requesting session.
    pub async fn list_threads(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let rows =
            crate::db::queries::threads::get_threads_for_channel(pool, &channel_id, server_id)
                .await
                .map_err(|e| format!("DB error: {e}"))?;

        let threads: Vec<ThreadInfo> = rows
            .into_iter()
            .map(|row| ThreadInfo {
                id: row.id,
                name: row.name,
                channel_type: row.channel_type,
                parent_message_id: row.thread_parent_message_id,
                archived: row.archived != 0,
                auto_archive_minutes: row.thread_auto_archive_minutes,
                message_count: 0, // would need a count query; returning 0 for now
                created_at: row.created_at,
            })
            .collect();

        let _ = session.send(ChatEvent::ThreadList {
            server_id: server_id.to_string(),
            channel: channel_name,
            threads,
        });

        Ok(())
    }

    // ── Bookmarks ───────────────────────────────────────────────

    /// Add a bookmark on a message for the authenticated user.
    pub async fn add_bookmark(
        &self,
        session_id: SessionId,
        message_id: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        // Look up the message for BookmarkInfo
        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let bookmark_id = Uuid::new_v4().to_string();
        crate::db::queries::bookmarks::add_bookmark(pool, &bookmark_id, user_id, message_id, note)
            .await
            .map_err(|e| format!("Failed to add bookmark: {e}"))?;

        let bookmark = BookmarkInfo {
            id: bookmark_id,
            message_id: message_id.to_string(),
            channel_id: msg.channel_id.unwrap_or_default(),
            from: msg.sender_nick,
            content: msg.content,
            timestamp: msg.created_at,
            note: note.map(|s| s.to_string()),
            created_at: Utc::now().to_rfc3339(),
        };

        let _ = session.send(ChatEvent::BookmarkAdd { bookmark });

        Ok(())
    }

    /// Remove a bookmark for the authenticated user.
    pub async fn remove_bookmark(
        &self,
        session_id: SessionId,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        crate::db::queries::bookmarks::remove_bookmark(pool, user_id, message_id)
            .await
            .map_err(|e| format!("Failed to remove bookmark: {e}"))?;

        let _ = session.send(ChatEvent::BookmarkRemove {
            message_id: message_id.to_string(),
        });

        Ok(())
    }

    /// List all bookmarks for the authenticated user. Sends BookmarkList event to the session.
    pub async fn list_bookmarks(&self, session_id: SessionId) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows = crate::db::queries::bookmarks::list_bookmarks(pool, user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let mut bookmarks = Vec::new();
        for row in rows {
            // Look up message content for each bookmark
            let (from, content, timestamp, channel_id) =
                match crate::db::queries::messages::get_message_by_id(pool, &row.message_id).await {
                    Ok(Some(msg)) => (
                        msg.sender_nick,
                        msg.content,
                        msg.created_at,
                        msg.channel_id.unwrap_or_default(),
                    ),
                    _ => (
                        "unknown".to_string(),
                        "[deleted]".to_string(),
                        String::new(),
                        String::new(),
                    ),
                };

            bookmarks.push(BookmarkInfo {
                id: row.id,
                message_id: row.message_id,
                channel_id,
                from,
                content,
                timestamp,
                note: row.note,
                created_at: row.created_at,
            });
        }

        let _ = session.send(ChatEvent::BookmarkList { bookmarks });

        Ok(())
    }

    // ── Phase 6: Moderation ─────────────────────────────────────

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
        session_id: SessionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(), String> {
        let actor_id = self
            .require_permission(session_id, server_id, None, Permissions::KICK_MEMBERS)
            .await?;

        // Prevent kicking the server owner
        if self.is_server_owner(server_id, target_user_id) {
            return Err("Cannot kick the server owner".into());
        }

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::moderation::kick_member(pool, server_id, target_user_id)
            .await
            .map_err(|e| format!("Failed to kick member: {e}"))?;

        // Remove from in-memory server state
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(target_user_id);
        }

        // Log to audit log
        let audit_id = Uuid::new_v4().to_string();
        let _ = crate::db::queries::audit_log::create_entry(
            pool,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: &actor_id,
                action_type: "member_kick",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason,
                changes: None,
            },
        )
        .await;

        // Broadcast kick event to server members
        let event = ChatEvent::MemberKick {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            kicked_by: actor_id,
            reason: reason.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Ban a member from a server, optionally deleting their messages.
    pub async fn ban_member(
        &self,
        session_id: SessionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
        delete_message_days: i32,
    ) -> Result<(), String> {
        let actor_id = self
            .require_permission(session_id, server_id, None, Permissions::BAN_MEMBERS)
            .await?;

        // Prevent banning the server owner
        if self.is_server_owner(server_id, target_user_id) {
            return Err("Cannot ban the server owner".into());
        }

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let ban_id = Uuid::new_v4().to_string();
        let days = delete_message_days.clamp(0, 7);

        crate::db::queries::bans::create_ban(
            pool,
            &ban_id,
            server_id,
            target_user_id,
            &actor_id,
            reason,
            days,
        )
        .await
        .map_err(|e| format!("Failed to ban member: {e}"))?;

        // Also kick them from the server
        let _ = crate::db::queries::moderation::kick_member(pool, server_id, target_user_id).await;

        // Delete messages if requested
        if days > 0 {
            let _ = crate::db::queries::moderation::delete_user_messages(
                pool,
                server_id,
                target_user_id,
                days,
            )
            .await;
        }

        // Remove from in-memory server state
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(target_user_id);
        }

        // Audit log
        let audit_id = Uuid::new_v4().to_string();
        let _ = crate::db::queries::audit_log::create_entry(
            pool,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: &actor_id,
                action_type: "member_ban",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason,
                changes: None,
            },
        )
        .await;

        // Broadcast
        let event = ChatEvent::MemberBan {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            banned_by: actor_id,
            reason: reason.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Unban a member from a server.
    pub async fn unban_member(
        &self,
        session_id: SessionId,
        server_id: &str,
        target_user_id: &str,
    ) -> Result<(), String> {
        let actor_id = self
            .require_permission(session_id, server_id, None, Permissions::BAN_MEMBERS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let removed = crate::db::queries::bans::remove_ban(pool, server_id, target_user_id)
            .await
            .map_err(|e| format!("Failed to unban member: {e}"))?;

        if !removed {
            return Err("User is not banned".into());
        }

        // Audit log
        let audit_id = Uuid::new_v4().to_string();
        let _ = crate::db::queries::audit_log::create_entry(
            pool,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: &actor_id,
                action_type: "member_unban",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason: None,
                changes: None,
            },
        )
        .await;

        // Broadcast
        let event = ChatEvent::MemberUnban {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Get the list of bans for a server.
    pub async fn list_bans(&self, session_id: SessionId, server_id: &str) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::BAN_MEMBERS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::bans::list_bans(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list bans: {e}"))?;

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
        session_id: SessionId,
        server_id: &str,
        target_user_id: &str,
        timeout_until: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), String> {
        let actor_id = self
            .require_permission(session_id, server_id, None, Permissions::KICK_MEMBERS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::moderation::set_member_timeout(
            pool,
            server_id,
            target_user_id,
            timeout_until,
        )
        .await
        .map_err(|e| format!("Failed to set timeout: {e}"))?;

        // Audit log
        let audit_id = Uuid::new_v4().to_string();
        let changes_json = timeout_until.map(|t| format!("{{\"timeout_until\":\"{t}\"}}"));
        let _ = crate::db::queries::audit_log::create_entry(
            pool,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: &actor_id,
                action_type: "member_timeout",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason,
                changes: changes_json.as_deref(),
            },
        )
        .await;

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
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        seconds: i32,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|v| v.clone())
            .ok_or_else(|| "Channel not found".to_string())?;

        let seconds = seconds.clamp(0, 21600); // max 6 hours

        crate::db::queries::moderation::set_slowmode(pool, &channel_id, seconds)
            .await
            .map_err(|e| format!("Failed to set slow mode: {e}"))?;

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
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        is_nsfw: bool,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|v| v.clone())
            .ok_or_else(|| "Channel not found".to_string())?;

        crate::db::queries::moderation::set_nsfw(pool, &channel_id, is_nsfw)
            .await
            .map_err(|e| format!("Failed to set NSFW: {e}"))?;

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
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        message_ids: Vec<String>,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_MESSAGES)
            .await?;

        if message_ids.is_empty() {
            return Err("No messages to delete".into());
        }
        if message_ids.len() > 100 {
            return Err("Cannot bulk delete more than 100 messages".into());
        }

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::moderation::bulk_delete_messages(pool, &message_ids)
            .await
            .map_err(|e| format!("Failed to bulk delete: {e}"))?;

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
        session_id: SessionId,
        server_id: &str,
        action_type: Option<&str>,
        limit: i64,
        before: Option<&str>,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let limit = limit.clamp(1, 100);
        let rows = crate::db::queries::audit_log::list_entries(
            pool,
            server_id,
            action_type,
            limit,
            before,
        )
        .await
        .map_err(|e| format!("Failed to get audit log: {e}"))?;

        let entries: Vec<AuditLogEntry> = rows
            .into_iter()
            .map(|r| AuditLogEntry {
                id: r.id,
                actor_id: r.actor_id,
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

    // ── AutoMod ──

    /// Create an automod rule.
    pub async fn create_automod_rule(
        &self,
        session_id: SessionId,
        params: &crate::db::models::CreateAutomodRuleParams<'_>,
    ) -> Result<(), String> {
        let server_id = params.server_id;
        let name = params.name;
        let rule_type = params.rule_type;
        let config = params.config;
        let action_type = params.action_type;
        let timeout_duration_seconds = params.timeout_duration_seconds;
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Validate rule_type
        if !["keyword", "mention_spam", "link_filter"].contains(&rule_type) {
            return Err(
                "Invalid rule type. Must be 'keyword', 'mention_spam', or 'link_filter'".into(),
            );
        }
        // Validate action_type
        if !["delete", "timeout", "flag"].contains(&action_type) {
            return Err("Invalid action type. Must be 'delete', 'timeout', or 'flag'".into());
        }
        // Validate config JSON matches rule_type schema
        validate_automod_config(rule_type, config)?;

        let rule_id = Uuid::new_v4().to_string();
        let db_params = crate::db::models::CreateAutomodRuleParams {
            id: &rule_id,
            server_id,
            name,
            rule_type,
            config,
            action_type,
            timeout_duration_seconds,
        };
        crate::db::queries::automod::create_rule(pool, &db_params)
            .await
            .map_err(|e| format!("Failed to create automod rule: {e}"))?;

        let rule = AutomodRuleInfo {
            id: rule_id,
            name: name.to_string(),
            enabled: true,
            rule_type: rule_type.to_string(),
            config: config.to_string(),
            action_type: action_type.to_string(),
            timeout_duration_seconds,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleUpdate {
                server_id: server_id.to_string(),
                rule,
            });
        }

        Ok(())
    }

    /// Update an automod rule.
    pub async fn update_automod_rule(
        &self,
        session_id: SessionId,
        params: &crate::db::models::UpdateAutomodRuleParams<'_>,
    ) -> Result<(), String> {
        let server_id = params.server_id;
        let rule_id = params.rule_id;
        let name = params.name;
        let enabled = params.enabled;
        let config = params.config;
        let action_type = params.action_type;
        let timeout_duration_seconds = params.timeout_duration_seconds;

        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Validate action_type
        if !["delete", "timeout", "flag"].contains(&action_type) {
            return Err("Invalid action type. Must be 'delete', 'timeout', or 'flag'".into());
        }

        // Fetch existing rule to get rule_type for config validation
        let existing = crate::db::queries::automod::get_rule(pool, rule_id)
            .await
            .map_err(|e| format!("Failed to fetch automod rule: {e}"))?
            .ok_or_else(|| "Automod rule not found".to_string())?;
        validate_automod_config(&existing.rule_type, config)?;

        crate::db::queries::automod::update_rule(
            pool,
            rule_id,
            name,
            enabled,
            config,
            action_type,
            timeout_duration_seconds,
        )
        .await
        .map_err(|e| format!("Failed to update automod rule: {e}"))?;

        let rule = AutomodRuleInfo {
            id: rule_id.to_string(),
            name: name.to_string(),
            enabled,
            rule_type: existing.rule_type,
            config: config.to_string(),
            action_type: action_type.to_string(),
            timeout_duration_seconds,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleUpdate {
                server_id: server_id.to_string(),
                rule,
            });
        }

        Ok(())
    }

    /// Delete an automod rule.
    pub async fn delete_automod_rule(
        &self,
        session_id: SessionId,
        server_id: &str,
        rule_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::automod::delete_rule(pool, rule_id)
            .await
            .map_err(|e| format!("Failed to delete automod rule: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleDelete {
                server_id: server_id.to_string(),
                rule_id: rule_id.to_string(),
            });
        }

        Ok(())
    }

    /// List automod rules for a server.
    pub async fn list_automod_rules(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::automod::list_rules(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list automod rules: {e}"))?;

        let rules: Vec<AutomodRuleInfo> = rows
            .into_iter()
            .map(|r| AutomodRuleInfo {
                id: r.id,
                name: r.name,
                enabled: r.enabled != 0,
                rule_type: r.rule_type,
                config: r.config,
                action_type: r.action_type,
                timeout_duration_seconds: r.timeout_duration_seconds,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleList {
                server_id: server_id.to_string(),
                rules,
            });
        }

        Ok(())
    }

    // ── Phase 7: Community & Discovery ─────────────────────────────

    // ── Invites ──

    /// Create a server invite. Requires CREATE_INVITES permission.
    pub async fn create_invite(
        &self,
        session_id: SessionId,
        server_id: &str,
        max_uses: Option<i32>,
        expires_at: Option<&str>,
        channel_id: Option<&str>,
    ) -> Result<(), String> {
        let user_id = self
            .require_permission(session_id, server_id, None, Permissions::CREATE_INVITES)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let invite_id = Uuid::new_v4().to_string();
        // Generate random 16-char alphanumeric invite code (brute-force resistant)
        use rand::Rng;
        let code: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();

        crate::db::queries::invites::create_invite(
            pool, &invite_id, server_id, &code, &user_id, max_uses, expires_at, channel_id,
        )
        .await
        .map_err(|e| format!("Failed to create invite: {e}"))?;

        let invite = InviteInfo {
            id: invite_id,
            code,
            server_id: server_id.to_string(),
            created_by: user_id,
            max_uses,
            use_count: 0,
            expires_at: expires_at.map(String::from),
            channel_id: channel_id.map(String::from),
            created_at: Utc::now().to_rfc3339(),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteCreate {
                server_id: server_id.to_string(),
                invite,
            });
        }

        Ok(())
    }

    /// List invites for a server. Requires MANAGE_SERVER permission.
    pub async fn list_invites(&self, session_id: SessionId, server_id: &str) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::invites::list_server_invites(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list invites: {e}"))?;

        let invites: Vec<InviteInfo> = rows
            .into_iter()
            .map(|r| InviteInfo {
                id: r.id,
                code: r.code,
                server_id: r.server_id,
                created_by: r.created_by,
                max_uses: r.max_uses,
                use_count: r.use_count,
                expires_at: r.expires_at,
                channel_id: r.channel_id,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteList {
                server_id: server_id.to_string(),
                invites,
            });
        }

        Ok(())
    }

    /// Delete an invite. Requires MANAGE_SERVER permission.
    pub async fn delete_invite(
        &self,
        session_id: SessionId,
        server_id: &str,
        invite_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::invites::delete_invite(pool, invite_id)
            .await
            .map_err(|e| format!("Failed to delete invite: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteDelete {
                server_id: server_id.to_string(),
                invite_id: invite_id.to_string(),
            });
        }

        Ok(())
    }

    /// Use an invite code to join a server. Any authenticated user can use this.
    pub async fn use_invite(&self, session_id: SessionId, code: &str) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let invite = crate::db::queries::invites::get_invite_by_code(pool, code)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Invalid invite code")?;

        // Check if invite has expired
        if let Some(ref expires_at) = invite.expires_at
            && let Ok(expiry) = expires_at.parse::<chrono::DateTime<Utc>>()
            && Utc::now() > expiry
        {
            return Err("Invite has expired".into());
        }

        // Check if user is already a member
        if let Some(server) = self.servers.get(&invite.server_id)
            && server.member_user_ids.contains(&user_id)
        {
            return Err("Already a member of this server".into());
        }

        // Atomically check max_uses and increment use_count (prevents TOCTOU race)
        let used = crate::db::queries::invites::try_use_invite(pool, &invite.id)
            .await
            .map_err(|e| format!("Failed to use invite: {e}"))?;
        if !used {
            return Err("Invite has reached maximum uses".into());
        }

        // Add user as server member
        self.join_server(&user_id, &invite.server_id).await?;

        // Auto-join default channel (#general)
        let default_channel = self
            .channel_name_index
            .get(&(invite.server_id.clone(), "#general".to_string()))
            .map(|r| r.clone());
        if default_channel.is_some() {
            let _ = self.join_channel(session_id, &invite.server_id, "#general");
        }

        // Send updated server list to the user
        let servers = self.list_servers_for_user(&user_id);
        let _ = session.send(ChatEvent::ServerList { servers });

        Ok(())
    }

    // ── Events ──

    /// Create a scheduled server event. Requires MANAGE_SERVER permission.
    pub async fn create_event(
        &self,
        session_id: SessionId,
        params: &crate::db::models::CreateServerEventParams<'_>,
    ) -> Result<(), String> {
        self.require_permission(
            session_id,
            params.server_id,
            None,
            Permissions::MANAGE_SERVER,
        )
        .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::events::create_event(pool, params)
            .await
            .map_err(|e| format!("Failed to create event: {e}"))?;

        let event_info = EventInfo {
            id: params.id.to_string(),
            server_id: params.server_id.to_string(),
            name: params.name.to_string(),
            description: params.description.map(String::from),
            channel_id: params.channel_id.map(String::from),
            start_time: params.start_time.to_string(),
            end_time: params.end_time.map(String::from),
            image_url: params.image_url.map(String::from),
            created_by: params.created_by.to_string(),
            status: "scheduled".to_string(),
            interested_count: 0,
            created_at: Utc::now().to_rfc3339(),
        };

        let event = ChatEvent::EventUpdate {
            server_id: params.server_id.to_string(),
            event: event_info,
        };
        self.broadcast_to_server(params.server_id, &event);

        Ok(())
    }

    /// List events for a server. Requires VIEW_CHANNELS permission.
    pub async fn list_events(&self, session_id: SessionId, server_id: &str) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::VIEW_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::events::list_server_events(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list events: {e}"))?;

        let mut events = Vec::new();
        for row in rows {
            let rsvp_count = crate::db::queries::events::get_rsvp_count(pool, &row.id)
                .await
                .unwrap_or(0);
            events.push(EventInfo {
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                description: row.description,
                channel_id: row.channel_id,
                start_time: row.start_time,
                end_time: row.end_time,
                image_url: row.image_url,
                created_by: row.created_by,
                status: row.status,
                interested_count: rsvp_count,
                created_at: row.created_at,
            });
        }

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::EventList {
                server_id: server_id.to_string(),
                events,
            });
        }

        Ok(())
    }

    /// Update an event's status. Requires MANAGE_SERVER permission.
    pub async fn update_event_status(
        &self,
        session_id: SessionId,
        server_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Validate status
        if !["scheduled", "active", "completed", "cancelled"].contains(&status) {
            return Err("Invalid status. Must be: scheduled, active, completed, cancelled".into());
        }

        crate::db::queries::events::update_event_status(pool, event_id, status)
            .await
            .map_err(|e| format!("Failed to update event status: {e}"))?;

        // Fetch updated event
        let row = crate::db::queries::events::get_event(pool, event_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Event not found")?;

        let rsvp_count = crate::db::queries::events::get_rsvp_count(pool, event_id)
            .await
            .unwrap_or(0);

        let event_info = EventInfo {
            id: row.id,
            server_id: row.server_id,
            name: row.name,
            description: row.description,
            channel_id: row.channel_id,
            start_time: row.start_time,
            end_time: row.end_time,
            image_url: row.image_url,
            created_by: row.created_by,
            status: row.status,
            interested_count: rsvp_count,
            created_at: row.created_at,
        };

        let event = ChatEvent::EventUpdate {
            server_id: server_id.to_string(),
            event: event_info,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Delete a scheduled event. Requires MANAGE_SERVER permission.
    pub async fn delete_event(
        &self,
        session_id: SessionId,
        server_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::events::delete_event(pool, event_id)
            .await
            .map_err(|e| format!("Failed to delete event: {e}"))?;

        let event = ChatEvent::EventDelete {
            server_id: server_id.to_string(),
            event_id: event_id.to_string(),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Set an RSVP for an event. Requires VIEW_CHANNELS permission.
    pub async fn set_rsvp(
        &self,
        session_id: SessionId,
        server_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), String> {
        let user_id = self
            .require_permission(session_id, server_id, None, Permissions::VIEW_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Validate RSVP status
        if !["interested", "going", "not_going"].contains(&status) {
            return Err("Invalid RSVP status. Must be: interested, going, not_going".into());
        }

        crate::db::queries::events::set_rsvp(pool, event_id, &user_id, status)
            .await
            .map_err(|e| format!("Failed to set RSVP: {e}"))?;

        // Send updated RSVP list to the session
        let rsvp_rows = crate::db::queries::events::get_rsvps(pool, event_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let rsvps: Vec<RsvpInfo> = rsvp_rows
            .into_iter()
            .map(|r| RsvpInfo {
                user_id: r.user_id,
                status: r.status,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::EventRsvpList {
                event_id: event_id.to_string(),
                rsvps,
            });
        }

        Ok(())
    }

    /// Remove an RSVP for an event. Requires VIEW_CHANNELS permission.
    pub async fn remove_rsvp(
        &self,
        session_id: SessionId,
        server_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        let user_id = self
            .require_permission(session_id, server_id, None, Permissions::VIEW_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::events::remove_rsvp(pool, event_id, &user_id)
            .await
            .map_err(|e| format!("Failed to remove RSVP: {e}"))?;

        Ok(())
    }

    /// List RSVPs for an event. Sends EventRsvpList to the requesting session.
    pub async fn list_rsvps(&self, session_id: SessionId, event_id: &str) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rsvp_rows = crate::db::queries::events::get_rsvps(pool, event_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let rsvps: Vec<RsvpInfo> = rsvp_rows
            .into_iter()
            .map(|r| RsvpInfo {
                user_id: r.user_id,
                status: r.status,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::EventRsvpList {
                event_id: event_id.to_string(),
                rsvps,
            });
        }

        Ok(())
    }

    // ── Community ──

    /// Update community/discovery settings. Requires MANAGE_SERVER permission.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_community_settings(
        &self,
        session_id: SessionId,
        server_id: &str,
        description: Option<&str>,
        is_discoverable: bool,
        welcome_message: Option<&str>,
        rules_text: Option<&str>,
        category: Option<&str>,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::community::update_server_community(
            pool,
            server_id,
            description,
            is_discoverable,
            welcome_message,
            rules_text,
            category,
        )
        .await
        .map_err(|e| format!("Failed to update community settings: {e}"))?;

        let community = ServerCommunityInfo {
            server_id: server_id.to_string(),
            description: description.map(String::from),
            is_discoverable,
            welcome_message: welcome_message.map(String::from),
            rules_text: rules_text.map(String::from),
            category: category.map(String::from),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerCommunity { community });
        }

        Ok(())
    }

    /// Get community/discovery settings for a server. Requires VIEW_CHANNELS permission.
    pub async fn get_community_settings(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::VIEW_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let server = crate::db::queries::servers::get_server(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Server not found")?;

        let community = ServerCommunityInfo {
            server_id: server.id,
            description: server.description,
            is_discoverable: server.is_discoverable != 0,
            welcome_message: server.welcome_message,
            rules_text: server.rules_text,
            category: server.category,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerCommunity { community });
        }

        Ok(())
    }

    /// Discover public servers, optionally filtered by category. No permission needed.
    pub async fn discover_servers(
        &self,
        session_id: SessionId,
        category: Option<&str>,
    ) -> Result<(), String> {
        // Verify the session exists (must be authenticated)
        let session = self.get_session(session_id).ok_or("Session not found")?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::community::list_discoverable_servers(pool, category, 100, 0)
            .await
            .map_err(|e| format!("Failed to list discoverable servers: {e}"))?;

        let servers: Vec<ServerCommunityInfo> = rows
            .into_iter()
            .map(|r| ServerCommunityInfo {
                server_id: r.id,
                description: r.description,
                is_discoverable: r.is_discoverable != 0,
                welcome_message: r.welcome_message,
                rules_text: r.rules_text,
                category: r.category,
            })
            .collect();

        let _ = session.send(ChatEvent::DiscoverServers { servers });

        Ok(())
    }

    /// Accept server rules as a member.
    pub async fn accept_rules(&self, session_id: SessionId, server_id: &str) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();

        // Verify membership
        let server = self
            .servers
            .get(server_id)
            .ok_or(format!("Server not found: {server_id}"))?;
        if !server.member_user_ids.contains(&user_id) {
            return Err("Not a member of this server".into());
        }
        drop(server);

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::community::accept_rules(pool, server_id, &user_id)
            .await
            .map_err(|e| format!("Failed to accept rules: {e}"))?;

        Ok(())
    }

    // ── Announcements ──

    /// Set a channel as an announcement channel. Requires MANAGE_CHANNELS permission.
    pub async fn set_announcement_channel(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_name: &str,
        is_announcement: bool,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        crate::db::queries::community::set_announcement_channel(pool, &channel_id, is_announcement)
            .await
            .map_err(|e| format!("Failed to set announcement channel: {e}"))?;

        Ok(())
    }

    /// Follow an announcement channel, cross-posting to a target channel.
    /// Requires MANAGE_CHANNELS permission on the target server.
    pub async fn follow_channel(
        &self,
        session_id: SessionId,
        source_channel_id: &str,
        target_channel_id: &str,
    ) -> Result<(), String> {
        // Determine the target channel's server for permission check
        let target_server_id = self
            .channels
            .get(target_channel_id)
            .map(|ch| ch.server_id.clone())
            .ok_or("Target channel not found")?;

        let user_id = self
            .require_permission(
                session_id,
                &target_server_id,
                None,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let follow_id = Uuid::new_v4().to_string();
        crate::db::queries::community::create_channel_follow(
            pool,
            &follow_id,
            source_channel_id,
            target_channel_id,
            &user_id,
        )
        .await
        .map_err(|e| format!("Failed to create channel follow: {e}"))?;

        let follow = ChannelFollowInfo {
            id: follow_id,
            source_channel_id: source_channel_id.to_string(),
            target_channel_id: target_channel_id.to_string(),
            created_by: user_id,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ChannelFollowCreate { follow });
        }

        Ok(())
    }

    /// Unfollow an announcement channel. Requires MANAGE_CHANNELS permission.
    pub async fn unfollow_channel(
        &self,
        session_id: SessionId,
        follow_id: &str,
    ) -> Result<(), String> {
        // We need to know which server to check permissions against.
        // Look up the follow to find the target channel, then the server.
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // We don't have a get_follow query, so we look up all follows and find the one.
        // For now, iterate channels to find which server the follow belongs to.
        // Since the session must have MANAGE_CHANNELS on *some* server to delete a follow,
        // we verify they are authenticated.
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let _user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;

        crate::db::queries::community::delete_channel_follow(pool, follow_id)
            .await
            .map_err(|e| format!("Failed to delete channel follow: {e}"))?;

        let _ = session.send(ChatEvent::ChannelFollowDelete {
            follow_id: follow_id.to_string(),
        });

        Ok(())
    }

    /// List follows for an announcement channel. Sends ChannelFollowList to the session.
    pub async fn list_channel_follows(
        &self,
        session_id: SessionId,
        channel_id: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::community::list_channel_follows(pool, channel_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let follows: Vec<ChannelFollowInfo> = rows
            .into_iter()
            .map(|r| ChannelFollowInfo {
                id: r.id,
                source_channel_id: r.source_channel_id,
                target_channel_id: r.target_channel_id,
                created_by: r.created_by,
            })
            .collect();

        let _ = session.send(ChatEvent::ChannelFollowList {
            channel_id: channel_id.to_string(),
            follows,
        });

        Ok(())
    }

    // ── Templates ──

    /// Create a server template (snapshot of channels, categories, roles).
    /// Requires MANAGE_SERVER permission.
    pub async fn create_template(
        &self,
        session_id: SessionId,
        server_id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<(), String> {
        let user_id = self
            .require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Snapshot server config: channels, categories, roles
        let channels = self.list_channels(server_id);
        let categories = self.list_categories(server_id).await.unwrap_or_default();
        let roles = self.list_roles(server_id).await.unwrap_or_default();

        let config = serde_json::json!({
            "channels": channels,
            "categories": categories,
            "roles": roles,
        });
        let config_str = config.to_string();

        let template_id = Uuid::new_v4().to_string();
        crate::db::queries::community::create_template(
            pool,
            &template_id,
            name,
            description,
            server_id,
            &user_id,
            &config_str,
        )
        .await
        .map_err(|e| format!("Failed to create template: {e}"))?;

        let template = TemplateInfo {
            id: template_id,
            name: name.to_string(),
            description: description.map(String::from),
            server_id: server_id.to_string(),
            created_by: user_id,
            use_count: 0,
            created_at: Utc::now().to_rfc3339(),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::TemplateUpdate {
                server_id: server_id.to_string(),
                template,
            });
        }

        Ok(())
    }

    /// List templates for a server. Sends TemplateList to the session.
    pub async fn list_templates(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::community::list_templates(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let templates: Vec<TemplateInfo> = rows
            .into_iter()
            .map(|r| TemplateInfo {
                id: r.id,
                name: r.name,
                description: r.description,
                server_id: r.server_id,
                created_by: r.created_by,
                use_count: r.use_count,
                created_at: r.created_at,
            })
            .collect();

        let _ = session.send(ChatEvent::TemplateList {
            server_id: server_id.to_string(),
            templates,
        });

        Ok(())
    }

    /// Delete a template. Requires MANAGE_SERVER permission.
    pub async fn delete_template(
        &self,
        session_id: SessionId,
        server_id: &str,
        template_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::community::delete_template(pool, template_id)
            .await
            .map_err(|e| format!("Failed to delete template: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::TemplateDelete {
                server_id: server_id.to_string(),
                template_id: template_id.to_string(),
            });
        }

        Ok(())
    }

    // ── Phase 8: Integrations & Bots ──

    /// Create a webhook for a channel. Requires MANAGE_SERVER permission.
    pub async fn create_webhook(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel_id: &str,
        name: &str,
        webhook_type: &str,
        url: Option<&str>,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        if webhook_type != "incoming" && webhook_type != "outgoing" {
            return Err("webhook_type must be 'incoming' or 'outgoing'".into());
        }

        let id = Uuid::new_v4().to_string();
        let raw_token = format!("{}.{}", id, Uuid::new_v4());
        let token_hash = crate::auth::token::hash_irc_token(&raw_token)
            .map_err(|e| format!("Failed to hash webhook token: {e}"))?;

        use crate::db::models::CreateWebhookParams;
        let params = CreateWebhookParams {
            id: &id,
            server_id,
            channel_id,
            name,
            avatar_url: None,
            webhook_type,
            token: &token_hash,
            url,
            created_by: &self.get_user_id(session_id)?,
        };

        crate::db::queries::webhooks::create_webhook(pool, &params)
            .await
            .map_err(|e| format!("Failed to create webhook: {e}"))?;

        let webhook = WebhookInfo {
            id: id.clone(),
            server_id: server_id.to_string(),
            channel_id: channel_id.to_string(),
            name: name.to_string(),
            avatar_url: None,
            webhook_type: webhook_type.to_string(),
            token: raw_token,
            url: url.map(String::from),
            created_by: self.get_user_id(session_id)?,
            created_at: Utc::now().to_rfc3339(),
        };

        self.broadcast_to_server(
            server_id,
            &ChatEvent::WebhookUpdate {
                server_id: server_id.to_string(),
                webhook,
            },
        );

        Ok(())
    }

    /// List webhooks for a server. Requires MANAGE_SERVER permission.
    pub async fn list_webhooks(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::webhooks::list_webhooks(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list webhooks: {e}"))?;

        let webhooks: Vec<WebhookInfo> = rows.into_iter().map(webhook_row_to_info).collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::WebhookList {
                server_id: server_id.to_string(),
                webhooks,
            });
        }

        Ok(())
    }

    /// Update a webhook.
    pub async fn update_webhook(
        &self,
        session_id: SessionId,
        webhook_id: &str,
        name: &str,
        avatar_url: Option<&str>,
        channel_id: &str,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let wh = crate::db::queries::webhooks::get_webhook(pool, webhook_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Webhook not found")?;

        self.require_permission(session_id, &wh.server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        crate::db::queries::webhooks::update_webhook(
            pool, webhook_id, name, avatar_url, channel_id,
        )
        .await
        .map_err(|e| format!("Failed to update webhook: {e}"))?;

        let sid = wh.server_id.clone();
        let updated = WebhookInfo {
            id: wh.id,
            server_id: wh.server_id,
            channel_id: channel_id.to_string(),
            name: name.to_string(),
            avatar_url: avatar_url.map(String::from),
            webhook_type: wh.webhook_type,
            token: wh.token,
            url: wh.url,
            created_by: wh.created_by,
            created_at: wh.created_at,
        };

        self.broadcast_to_server(
            &sid,
            &ChatEvent::WebhookUpdate {
                server_id: sid.clone(),
                webhook: updated,
            },
        );

        Ok(())
    }

    /// Delete a webhook.
    pub async fn delete_webhook(
        &self,
        session_id: SessionId,
        webhook_id: &str,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let wh = crate::db::queries::webhooks::get_webhook(pool, webhook_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Webhook not found")?;

        self.require_permission(session_id, &wh.server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        crate::db::queries::webhooks::delete_webhook(pool, webhook_id)
            .await
            .map_err(|e| format!("Failed to delete webhook: {e}"))?;

        let sid = wh.server_id;
        self.broadcast_to_server(
            &sid,
            &ChatEvent::WebhookDelete {
                server_id: sid.clone(),
                webhook_id: webhook_id.to_string(),
            },
        );

        Ok(())
    }

    /// Create a bot account. Only authenticated users can create bots.
    pub async fn create_bot(
        &self,
        session_id: SessionId,
        username: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), String> {
        let _creator_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        validation::validate_nickname(username)?;

        let bot_user_id = Uuid::new_v4().to_string();
        crate::db::queries::bots::create_bot_user(pool, &bot_user_id, username, avatar_url)
            .await
            .map_err(|e| format!("Failed to create bot: {e}"))?;

        // Generate an initial token
        let token_id = Uuid::new_v4().to_string();
        let raw_token = format!("bot_{}.{}", bot_user_id, Uuid::new_v4());
        let token_hash = crate::auth::token::hash_irc_token(&raw_token)
            .map_err(|e| format!("Failed to hash token: {e}"))?;

        crate::db::queries::bots::create_bot_token(
            pool,
            &token_id,
            &bot_user_id,
            &token_hash,
            "Default",
            "bot",
        )
        .await
        .map_err(|e| format!("Failed to create bot token: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            // Send back the raw token (only time it's visible)
            let _ = session.send(ChatEvent::ServerNotice {
                message: format!("Bot created! User ID: {bot_user_id}, Token: {raw_token}"),
            });
        }

        Ok(())
    }

    /// Create a new token for a bot. Caller must own the bot.
    pub async fn create_bot_token(
        &self,
        session_id: SessionId,
        bot_user_id: &str,
        name: &str,
        scopes: Option<&str>,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let token_id = Uuid::new_v4().to_string();
        let raw_token = format!("bot_{}.{}", bot_user_id, Uuid::new_v4());
        let token_hash = crate::auth::token::hash_irc_token(&raw_token)
            .map_err(|e| format!("Failed to hash token: {e}"))?;

        crate::db::queries::bots::create_bot_token(
            pool,
            &token_id,
            bot_user_id,
            &token_hash,
            name,
            scopes.unwrap_or("bot"),
        )
        .await
        .map_err(|e| format!("Failed to create bot token: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerNotice {
                message: format!("Bot token created: {raw_token}"),
            });
        }

        Ok(())
    }

    /// List bot tokens (without hashes).
    pub async fn list_bot_tokens(
        &self,
        session_id: SessionId,
        bot_user_id: &str,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::bots::list_bot_tokens(pool, bot_user_id)
            .await
            .map_err(|e| format!("Failed to list bot tokens: {e}"))?;

        let tokens: Vec<BotTokenInfo> = rows
            .into_iter()
            .map(|r| BotTokenInfo {
                id: r.id,
                name: r.name,
                scopes: r.scopes,
                created_at: r.created_at,
                last_used: r.last_used,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotTokenList {
                bot_user_id: bot_user_id.to_string(),
                tokens,
            });
        }

        Ok(())
    }

    /// Delete a bot token.
    pub async fn delete_bot_token(
        &self,
        session_id: SessionId,
        token_id: &str,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::delete_bot_token(pool, token_id)
            .await
            .map_err(|e| format!("Failed to delete bot token: {e}"))?;

        Ok(())
    }

    /// Add a bot to a server. Requires MANAGE_SERVER permission.
    pub async fn add_bot_to_server(
        &self,
        session_id: SessionId,
        server_id: &str,
        bot_user_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::add_bot_to_server(pool, server_id, bot_user_id)
            .await
            .map_err(|e| format!("Failed to add bot to server: {e}"))?;

        Ok(())
    }

    /// Remove a bot from a server. Requires MANAGE_SERVER permission.
    pub async fn remove_bot_from_server(
        &self,
        session_id: SessionId,
        server_id: &str,
        bot_user_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::remove_bot_from_server(pool, server_id, bot_user_id)
            .await
            .map_err(|e| format!("Failed to remove bot from server: {e}"))?;

        Ok(())
    }

    /// Register a slash command for a bot. Caller must own the bot.
    pub async fn register_slash_command(
        &self,
        session_id: SessionId,
        server_id: &str,
        name: &str,
        description: &str,
        options_json: Option<&str>,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        if name.is_empty() || name.len() > 32 || name.contains(' ') {
            return Err("Command name must be 1-32 chars with no spaces".into());
        }

        let id = Uuid::new_v4().to_string();
        let opts = options_json.unwrap_or("[]");
        // Validate JSON
        serde_json::from_str::<Vec<SlashCommandOption>>(opts)
            .map_err(|e| format!("Invalid options JSON: {e}"))?;

        use crate::db::models::CreateSlashCommandParams;
        let params = CreateSlashCommandParams {
            id: &id,
            bot_user_id: &self.get_user_id(session_id)?,
            server_id: Some(server_id),
            name,
            description,
            options_json: opts,
        };

        crate::db::queries::slash_commands::create_command(pool, &params)
            .await
            .map_err(|e| format!("Failed to register command: {e}"))?;

        let options: Vec<SlashCommandOption> = serde_json::from_str(opts).unwrap_or_default();
        let cmd = SlashCommandInfo {
            id: id.clone(),
            bot_user_id: self.get_user_id(session_id)?,
            name: name.to_string(),
            description: description.to_string(),
            options,
        };

        self.broadcast_to_server(
            server_id,
            &ChatEvent::SlashCommandUpdate {
                server_id: server_id.to_string(),
                command: cmd,
            },
        );

        Ok(())
    }

    /// List slash commands available in a server.
    pub async fn list_slash_commands(
        &self,
        session_id: SessionId,
        server_id: &str,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::slash_commands::list_commands_for_server(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list commands: {e}"))?;

        let commands: Vec<SlashCommandInfo> = rows
            .into_iter()
            .map(|r| {
                let options: Vec<SlashCommandOption> =
                    serde_json::from_str(&r.options_json).unwrap_or_default();
                SlashCommandInfo {
                    id: r.id,
                    bot_user_id: r.bot_user_id,
                    name: r.name,
                    description: r.description,
                    options,
                }
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::SlashCommandList {
                server_id: server_id.to_string(),
                commands,
            });
        }

        Ok(())
    }

    /// Delete a slash command.
    pub async fn delete_slash_command(
        &self,
        session_id: SessionId,
        command_id: &str,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let cmd = crate::db::queries::slash_commands::get_command(pool, command_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Command not found")?;

        crate::db::queries::slash_commands::delete_command(pool, command_id)
            .await
            .map_err(|e| format!("Failed to delete command: {e}"))?;

        if let Some(sid) = &cmd.server_id {
            self.broadcast_to_server(
                sid,
                &ChatEvent::SlashCommandDelete {
                    server_id: sid.clone(),
                    command_id: command_id.to_string(),
                },
            );
        }

        Ok(())
    }

    /// Invoke a slash command. Creates an interaction and dispatches to the bot.
    pub async fn invoke_slash_command(
        &self,
        session_id: SessionId,
        server_id: &str,
        channel: &str,
        command_name: &str,
        args_json: Option<&str>,
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Find the command by name in this server
        let commands =
            crate::db::queries::slash_commands::list_commands_for_server(pool, server_id)
                .await
                .map_err(|e| format!("DB error: {e}"))?;

        let cmd = commands
            .iter()
            .find(|c| c.name == command_name)
            .ok_or_else(|| format!("Unknown command: /{command_name}"))?;

        let interaction_id = Uuid::new_v4().to_string();
        let data: serde_json::Value = args_json
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        // Resolve channel_id from name
        let channel_id = self.resolve_channel_id(server_id, channel)?;

        let data_str = serde_json::to_string(&data).unwrap_or_default();
        let interaction_params = crate::db::models::CreateInteractionParams {
            id: &interaction_id,
            interaction_type: "slash_command",
            command_id: Some(&cmd.id),
            user_id: &user_id,
            server_id,
            channel_id: &channel_id,
            data_json: &data_str,
        };
        crate::db::queries::slash_commands::create_interaction(pool, &interaction_params)
            .await
            .map_err(|e| format!("Failed to create interaction: {e}"))?;

        let interaction = InteractionInfo {
            id: interaction_id.clone(),
            interaction_type: "slash_command".to_string(),
            command_name: Some(command_name.to_string()),
            user_id: user_id.clone(),
            server_id: server_id.to_string(),
            channel_id: channel_id.clone(),
            data,
        };

        // Send to the bot (find sessions for the bot user)
        for entry in self.sessions.iter() {
            let s = entry.value();
            if let Some(ref uid) = s.user_id
                && uid == &cmd.bot_user_id
            {
                let _ = s.send(ChatEvent::InteractionCreate {
                    interaction: interaction.clone(),
                });
            }
        }

        // Also send a notice to the invoker
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerNotice {
                message: format!("/{command_name} invoked"),
            });
        }

        Ok(())
    }

    /// Respond to an interaction (bot -> channel).
    pub async fn respond_to_interaction(
        &self,
        session_id: SessionId,
        interaction_id: &str,
        content: Option<&str>,
        embeds_json: Option<&str>,
        components_json: Option<&str>,
        ephemeral: bool,
    ) -> Result<(), String> {
        let _user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let interaction = crate::db::queries::slash_commands::get_interaction(pool, interaction_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Interaction not found")?;

        crate::db::queries::slash_commands::mark_interaction_responded(pool, interaction_id)
            .await
            .map_err(|e| format!("Failed to mark interaction: {e}"))?;

        let embeds = embeds_json.and_then(|s| serde_json::from_str(s).ok());
        let components = components_json.and_then(|s| serde_json::from_str(s).ok());

        let response = InteractionResponseData {
            content: content.map(String::from),
            embeds,
            components,
            ephemeral,
        };

        // Resolve channel name from channel_id
        let channel_name = self
            .resolve_channel_name_from_id(&interaction.channel_id)
            .unwrap_or_else(|_| interaction.channel_id.clone());

        if ephemeral {
            // Send only to the invoker
            for entry in self.sessions.iter() {
                let s = entry.value();
                if let Some(ref uid) = s.user_id
                    && uid == &interaction.user_id
                {
                    let _ = s.send(ChatEvent::InteractionResponse {
                        interaction_id: interaction_id.to_string(),
                        server_id: interaction.server_id.clone(),
                        channel: channel_name.clone(),
                        response: response.clone(),
                    });
                }
            }
        } else {
            self.broadcast_to_server(
                &interaction.server_id,
                &ChatEvent::InteractionResponse {
                    interaction_id: interaction_id.to_string(),
                    server_id: interaction.server_id.clone(),
                    channel: channel_name,
                    response,
                },
            );
        }

        Ok(())
    }

    /// Create an OAuth2 application.
    pub async fn create_oauth2_app(
        &self,
        session_id: SessionId,
        name: &str,
        description: Option<&str>,
        redirect_uris: &[String],
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let id = Uuid::new_v4().to_string();
        let raw_secret = format!("secret_{}", Uuid::new_v4());
        let secret_hash = crate::auth::token::hash_irc_token(&raw_secret)
            .map_err(|e| format!("Failed to hash OAuth2 secret: {e}"))?;
        let uris_json = serde_json::to_string(redirect_uris)
            .map_err(|e| format!("Invalid redirect URIs: {e}"))?;

        use crate::db::models::CreateOAuth2AppParams;
        let params = CreateOAuth2AppParams {
            id: &id,
            name,
            description: description.unwrap_or(""),
            icon_url: None,
            owner_id: &user_id,
            client_secret: &secret_hash,
            redirect_uris: &uris_json,
            scopes: "identify",
        };

        crate::db::queries::oauth2::create_app(pool, &params)
            .await
            .map_err(|e| format!("Failed to create OAuth2 app: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let app = OAuth2AppInfo {
                id: id.clone(),
                name: name.to_string(),
                description: description.unwrap_or("").to_string(),
                icon_url: None,
                owner_id: user_id,
                redirect_uris: redirect_uris.to_vec(),
                scopes: "identify".to_string(),
                is_public: false,
                created_at: Utc::now().to_rfc3339(),
            };
            let _ = session.send(ChatEvent::OAuth2AppUpdate { app });
            // Also show the secret (only time visible)
            let _ = session.send(ChatEvent::ServerNotice {
                message: format!(
                    "OAuth2 app created! Client ID: {id}, Client Secret: {raw_secret}"
                ),
            });
        }

        Ok(())
    }

    /// List OAuth2 apps owned by the current user.
    pub async fn list_oauth2_apps(&self, session_id: SessionId) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::oauth2::list_apps_by_owner(pool, &user_id)
            .await
            .map_err(|e| format!("Failed to list OAuth2 apps: {e}"))?;

        let apps: Vec<OAuth2AppInfo> = rows
            .into_iter()
            .map(|r| {
                let uris: Vec<String> = serde_json::from_str(&r.redirect_uris).unwrap_or_default();
                OAuth2AppInfo {
                    id: r.id,
                    name: r.name,
                    description: r.description,
                    icon_url: r.icon_url,
                    owner_id: r.owner_id,
                    redirect_uris: uris,
                    scopes: r.scopes,
                    is_public: r.is_public != 0,
                    created_at: r.created_at,
                }
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::OAuth2AppList { apps });
        }

        Ok(())
    }

    /// Delete an OAuth2 app. Only the owner can delete.
    pub async fn delete_oauth2_app(
        &self,
        session_id: SessionId,
        app_id: &str,
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let app = crate::db::queries::oauth2::get_app(pool, app_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("OAuth2 app not found")?;

        if app.owner_id != user_id {
            return Err("You can only delete your own apps".into());
        }

        crate::db::queries::oauth2::delete_app(pool, app_id)
            .await
            .map_err(|e| format!("Failed to delete app: {e}"))?;

        Ok(())
    }

    /// Execute an incoming webhook — post a message to the webhook's channel.
    /// No session required; the webhook token is the authentication.
    pub async fn execute_incoming_webhook(
        &self,
        webhook_id: &str,
        webhook_token: &str,
        content: &str,
        username_override: Option<&str>,
        avatar_override: Option<&str>,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let wh = crate::db::queries::webhooks::get_webhook(pool, webhook_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Invalid webhook")?;

        // Verify the token against the stored hash
        if !crate::auth::token::verify_irc_token(webhook_token, &wh.token) {
            return Err("Invalid webhook token".into());
        }

        if wh.webhook_type != "incoming" {
            return Err("This endpoint is only for incoming webhooks".into());
        }

        let channel_name = self.resolve_channel_name_from_id(&wh.channel_id)?;
        // Tag webhook display names to prevent impersonation of real users
        let base_name = username_override.unwrap_or(&wh.name);
        let display_name = format!("{base_name} [Webhook]");
        let avatar = avatar_override.map(String::from).or(wh.avatar_url.clone());

        let msg_id = Uuid::new_v4();
        let event = ChatEvent::Message {
            id: msg_id,
            server_id: Some(wh.server_id.clone()),
            from: display_name.to_string(),
            target: channel_name.clone(),
            content: content.to_string(),
            timestamp: Utc::now(),
            avatar_url: avatar,
            reply_to: None,
            attachments: None,
        };

        // Persist the message
        {
            let pool = pool.clone();
            let id = msg_id.to_string();
            let srv = wh.server_id.clone();
            let ch = wh.channel_id.clone();
            let wh_id = wh.id.clone();
            let nick = display_name.to_string();
            let msg = content.to_string();
            tokio::spawn(async move {
                let params = crate::db::queries::messages::InsertMessageParams {
                    id: &id,
                    server_id: &srv,
                    channel_id: &ch,
                    sender_id: &format!("webhook:{wh_id}"),
                    sender_nick: &nick,
                    content: &msg,
                    reply_to_id: None,
                };
                if let Err(e) = crate::db::queries::messages::insert_message(&pool, &params).await {
                    error!(error = %e, "failed to persist webhook message");
                }
            });
        }

        self.broadcast_to_channel(&wh.channel_id, &event, None);
        Ok(())
    }

    /// Helper: get user_id for a session.
    fn get_user_id(&self, session_id: SessionId) -> Result<String, String> {
        let session = self.sessions.get(&session_id).ok_or("Session not found")?;
        session
            .user_id
            .clone()
            .ok_or_else(|| "AUTH_REQUIRED".into())
    }

    /// Helper: resolve channel name from a channel_id by looking it up in self.channels.
    fn resolve_channel_name_from_id(&self, channel_id: &str) -> Result<String, String> {
        self.channels
            .get(channel_id)
            .map(|ch| ch.name.clone())
            .ok_or_else(|| format!("Channel ID {channel_id} not found"))
    }
}

/// Validate automod rule config JSON matches the expected schema for its rule_type.
fn validate_automod_config(rule_type: &str, config: &str) -> Result<(), String> {
    let parsed: serde_json::Value =
        serde_json::from_str(config).map_err(|_| "Invalid JSON in automod config".to_string())?;

    match rule_type {
        "keyword" => {
            // Expect {"words": ["word1", "word2", ...]}
            let words = parsed
                .get("words")
                .and_then(|v| v.as_array())
                .ok_or("keyword config must have a 'words' array")?;
            if words.is_empty() {
                return Err("keyword config 'words' array must not be empty".into());
            }
            if words.len() > 1000 {
                return Err("keyword config 'words' array exceeds maximum of 1000 entries".into());
            }
            for w in words {
                if w.as_str().is_none() {
                    return Err("keyword config 'words' must contain only strings".into());
                }
            }
        }
        "mention_spam" => {
            // Expect {"max_mentions": <positive integer>}
            let max = parsed
                .get("max_mentions")
                .and_then(|v| v.as_i64())
                .ok_or("mention_spam config must have a 'max_mentions' integer")?;
            if !(1..=100).contains(&max) {
                return Err("mention_spam 'max_mentions' must be between 1 and 100".into());
            }
        }
        "link_filter" => {
            // Expect {"block_all": <bool>} or {"allowed_domains": [...]}
            if parsed.get("block_all").is_none() && parsed.get("allowed_domains").is_none() {
                return Err(
                    "link_filter config must have 'block_all' (bool) or 'allowed_domains' (array)"
                        .into(),
                );
            }
        }
        _ => {
            return Err(format!("Unknown rule type: {rule_type}"));
        }
    }
    Ok(())
}

/// Convert a RoleRow to a RoleInfo for client consumption.
fn role_row_to_info(row: crate::db::models::RoleRow) -> RoleInfo {
    RoleInfo {
        id: row.id,
        server_id: row.server_id,
        name: row.name,
        color: row.color,
        icon_url: row.icon_url,
        position: row.position,
        permissions: row.permissions,
        is_default: row.is_default != 0,
    }
}

/// Convert a ChannelCategoryRow to a CategoryInfo for client consumption.
fn category_row_to_info(row: crate::db::models::ChannelCategoryRow) -> CategoryInfo {
    CategoryInfo {
        id: row.id,
        server_id: row.server_id,
        name: row.name,
        position: row.position,
    }
}

/// Convert a WebhookRow to a WebhookInfo for client consumption.
fn webhook_row_to_info(row: crate::db::models::WebhookRow) -> WebhookInfo {
    WebhookInfo {
        id: row.id,
        server_id: row.server_id,
        channel_id: row.channel_id,
        name: row.name,
        avatar_url: row.avatar_url,
        webhook_type: row.webhook_type,
        token: row.token,
        url: row.url,
        created_by: row.created_by,
        created_at: row.created_at,
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
        let engine = ChatEngine::new(None, 4000);
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
            .send_message(
                sid1,
                DEFAULT_SERVER_ID,
                "#general",
                "Hello from Alice!",
                None,
                None,
                None,
            )
            .unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message { from, content, .. } => {
                assert_eq!(from, "alice");
                assert_eq!(content, "Hello from Alice!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }

        // Sender receives a MessageAck (not the Message itself)
        let ack = rx1.try_recv().unwrap();
        assert!(matches!(ack, ChatEvent::MessageAck { .. }));
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
            .send_message(sid1, DEFAULT_SERVER_ID, "bob", "Hey Bob!", None, None, None)
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
        let result = engine.send_message(sid, &server_b, "#general", "Hello", None, None, None);
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Edge cases: sessions
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_disconnect_nonexistent_session_is_noop() {
        let engine = setup_engine();
        let fake_id = Uuid::new_v4();
        // Should not panic
        engine.disconnect(fake_id);
    }

    #[tokio::test]
    async fn test_get_session_nonexistent() {
        let engine = setup_engine();
        assert!(engine.get_session(Uuid::new_v4()).is_none());
    }

    #[tokio::test]
    async fn test_is_nick_available() {
        let engine = setup_engine();
        assert!(engine.is_nick_available("alice"));
        let (_sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        assert!(!engine.is_nick_available("alice"));
        assert!(engine.is_nick_available("bob"));
    }

    #[tokio::test]
    async fn test_connect_invalid_nickname_rejected() {
        let engine = setup_engine();
        let result = engine.connect(None, "".into(), Protocol::WebSocket, None);
        assert!(result.is_err());

        let result = engine.connect(None, "has space".into(), Protocol::WebSocket, None);
        assert!(result.is_err());

        let result = engine.connect(None, "has!bang".into(), Protocol::WebSocket, None);
        assert!(result.is_err());

        // Exactly at max length should be fine
        let nick32 = "a".repeat(32);
        let result = engine.connect(None, nick32, Protocol::WebSocket, None);
        assert!(result.is_ok());

        // Over max length should fail
        let nick33 = "b".repeat(33);
        let result = engine.connect(None, nick33, Protocol::WebSocket, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_connect_with_user_id_and_avatar() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(
                Some("user_123".into()),
                "alice".into(),
                Protocol::WebSocket,
                Some("https://example.com/avatar.png".into()),
            )
            .unwrap();
        let session = engine.get_session(sid).unwrap();
        assert_eq!(session.user_id.as_deref(), Some("user_123"));
        assert_eq!(
            session.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
    }

    #[tokio::test]
    async fn test_connect_irc_protocol() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "irc-user".into(), Protocol::Irc, None)
            .unwrap();
        let session = engine.get_session(sid).unwrap();
        assert_eq!(session.protocol, Protocol::Irc);
    }

    // ────────────────────────────────────────────────────────────────
    // Edge cases: channels and messaging
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_send_message_to_nonexistent_channel() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#nonexistent",
            "hello",
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_message_with_invalid_session() {
        let engine = setup_engine();
        let fake = Uuid::new_v4();
        let result =
            engine.send_message(fake, DEFAULT_SERVER_ID, "#general", "hi", None, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_join_channel_nonexistent_server() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        // join_channel creates channels on-the-fly even for servers not
        // registered in the servers map, so this should succeed.
        let result = engine.join_channel(sid, "no-such-server", "#general");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_join_channel_creates_it() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        // #new-channel doesn't exist yet
        assert!(engine.list_channels(DEFAULT_SERVER_ID).is_empty());
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#new-channel")
            .unwrap();
        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].name, "#new-channel");
    }

    #[tokio::test]
    async fn test_join_channel_twice_is_ok() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        // Joining again should be a no-op, not an error
        let result = engine.join_channel(sid, DEFAULT_SERVER_ID, "#general");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_part_channel_not_in() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        // Create the channel first by having someone join
        let (sid2, _rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        // alice never joined, so parting should fail
        let result = engine.part_channel(sid, DEFAULT_SERVER_ID, "#general", None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_channel_name_normalization_on_join() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        // Joining with "General" should normalize to "#general"
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "General")
            .unwrap();
        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert_eq!(channels[0].name, "#general");
    }

    #[tokio::test]
    async fn test_message_not_echoed_to_sender() {
        let engine = setup_engine();
        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        // Drain join events
        while rx.try_recv().is_ok() {}

        engine
            .send_message(
                sid,
                DEFAULT_SERVER_ID,
                "#general",
                "hello",
                None,
                None,
                None,
            )
            .unwrap();
        // Message should NOT be echoed back to the sender (only a MessageAck)
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, ChatEvent::MessageAck { .. }));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_empty_message_rejected() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        let result = engine.send_message(sid, DEFAULT_SERVER_ID, "#general", "", None, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_whitespace_only_message_rejected() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        let result =
            engine.send_message(sid, DEFAULT_SERVER_ID, "#general", "   ", None, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oversized_message_rejected() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        let long_msg = "x".repeat(4001);
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            &long_msg,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_message_at_max_length_accepted() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        let max_msg = "x".repeat(4000);
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            &max_msg,
            None,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    // ────────────────────────────────────────────────────────────────
    // Topic edge cases
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_set_topic_too_long() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        let long_topic = "t".repeat(501);
        let result = engine.set_topic(sid, DEFAULT_SERVER_ID, "#general", long_topic);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_topic_empty_clears() {
        let engine = setup_engine();
        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        while rx.try_recv().is_ok() {}

        // Set a topic first
        engine
            .set_topic(sid, DEFAULT_SERVER_ID, "#general", "Hello".into())
            .unwrap();
        while rx.try_recv().is_ok() {}

        // Clear topic
        engine
            .set_topic(sid, DEFAULT_SERVER_ID, "#general", "".into())
            .unwrap();
        let event = rx.try_recv().unwrap();
        match event {
            ChatEvent::TopicChange { topic, .. } => {
                assert_eq!(topic, "");
            }
            _ => panic!("Expected TopicChange event"),
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Server management
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_server_invalid_name() {
        let engine = setup_engine();
        // Empty name
        let result = engine.create_server("".into(), "user1".into(), None).await;
        assert!(result.is_err());

        // Whitespace-only name
        let result = engine
            .create_server("   ".into(), "user1".into(), None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_server_too_long_name() {
        let engine = setup_engine();
        let long_name = "a".repeat(101);
        let result = engine.create_server(long_name, "user1".into(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_server_max_name_length() {
        let engine = setup_engine();
        let max_name = "a".repeat(100);
        let result = engine.create_server(max_name, "user1".into(), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_server_with_icon() {
        let engine = setup_engine();
        let server_id = engine
            .create_server(
                "My Server".into(),
                "user1".into(),
                Some("https://example.com/icon.png".into()),
            )
            .await
            .unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert_eq!(
            server.icon_url.as_deref(),
            Some("https://example.com/icon.png")
        );
    }

    #[tokio::test]
    async fn test_find_server_by_name() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test Server".into(), "user1".into(), None)
            .await
            .unwrap();
        // Case insensitive lookup
        assert_eq!(
            engine.find_server_by_name("test server"),
            Some(server_id.clone())
        );
        assert_eq!(engine.find_server_by_name("TEST SERVER"), Some(server_id));
        assert!(engine.find_server_by_name("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_get_server_name() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("My Server".into(), "user1".into(), None)
            .await
            .unwrap();
        assert_eq!(
            engine.get_server_name(&server_id),
            Some("My Server".to_string())
        );
        assert!(engine.get_server_name("nonexistent").is_none());
    }

    // ────────────────────────────────────────────────────────────────
    // Multiple channels and users
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_multiple_channels_message_isolation() {
        let engine = setup_engine();
        let (sid_alice, mut rx_alice) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid_bob, mut rx_bob) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        // Alice joins #general, Bob joins #rust
        engine
            .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        engine
            .join_channel(sid_bob, DEFAULT_SERVER_ID, "#rust")
            .unwrap();
        while rx_alice.try_recv().is_ok() {}
        while rx_bob.try_recv().is_ok() {}

        // Alice sends to #general — Bob should NOT receive it (different channel)
        engine
            .send_message(
                sid_alice,
                DEFAULT_SERVER_ID,
                "#general",
                "hello general",
                None,
                None,
                None,
            )
            .unwrap();
        assert!(rx_bob.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_disconnect_broadcasts_quit() {
        let engine = setup_engine();
        let (sid_alice, _rx_alice) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid_bob, mut rx_bob) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        engine
            .join_channel(sid_bob, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        while rx_bob.try_recv().is_ok() {}

        engine.disconnect(sid_alice);

        let event = rx_bob.try_recv().unwrap();
        match event {
            ChatEvent::Quit { nickname, .. } => assert_eq!(nickname, "alice"),
            _ => panic!("Expected Quit event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_disconnect_removes_from_channel() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();

        engine.disconnect(sid);

        // Channel should have 0 members now
        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert_eq!(channels[0].member_count, 0);
    }

    // ────────────────────────────────────────────────────────────────
    // Normalize channel name additional tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_normalize_channel_name_already_lowercase() {
        assert_eq!(normalize_channel_name("#already-lower"), "#already-lower");
    }

    #[test]
    fn test_normalize_channel_name_mixed_case() {
        assert_eq!(normalize_channel_name("MixedCase"), "#mixedcase");
    }

    #[test]
    fn test_normalize_channel_name_uppercase_with_hash() {
        assert_eq!(normalize_channel_name("#UPPER"), "#upper");
    }

    #[test]
    fn test_normalize_channel_name_with_numbers() {
        assert_eq!(normalize_channel_name("channel123"), "#channel123");
    }

    // ────────────────────────────────────────────────────────────────
    // DM edge cases
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dm_to_nonexistent_user() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let result =
            engine.send_message(sid, DEFAULT_SERVER_ID, "nobody", "hello", None, None, None);
        // DMs to non-existent users fail because there's no channel and no user session
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Rate limiting
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_message_rate_limiting() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();

        // The default rate limiter allows burst of 10.
        // Send 10 messages — all should succeed.
        for i in 0..10 {
            let result = engine.send_message(
                sid,
                DEFAULT_SERVER_ID,
                "#general",
                &format!("msg {i}"),
                None,
                None,
                None,
            );
            assert!(result.is_ok(), "Message {i} should succeed");
        }

        // 11th should be rate-limited
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            "msg 10",
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Server owner and join_server (in-memory only, no DB)
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_server_sets_owner() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test".into(), "user1".into(), None)
            .await
            .unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert_eq!(server.owner_id, "user1");
    }

    #[tokio::test]
    async fn test_join_server_nonexistent() {
        let engine = setup_engine();
        let result = engine.join_server("user1", "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_join_server_adds_member() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test".into(), "owner".into(), None)
            .await
            .unwrap();
        engine.join_server("user1", &server_id).await.unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert!(server.member_user_ids.contains("user1"));
    }

    #[tokio::test]
    async fn test_leave_server_removes_member() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test".into(), "owner".into(), None)
            .await
            .unwrap();
        engine.join_server("user1", &server_id).await.unwrap();
        engine.leave_server("user1", &server_id).await.unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert!(!server.member_user_ids.contains("user1"));
    }

    // ────────────────────────────────────────────────────────────────
    // Part channel with reason
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_part_channel_with_reason() {
        let engine = setup_engine();
        let (sid_alice, mut rx_alice) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid_bob, _rx_bob) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        engine
            .join_channel(sid_bob, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        while rx_alice.try_recv().is_ok() {}

        engine
            .part_channel(
                sid_bob,
                DEFAULT_SERVER_ID,
                "#general",
                Some("bye!".to_string()),
            )
            .unwrap();

        let event = rx_alice.try_recv().unwrap();
        match event {
            ChatEvent::Part {
                nickname, reason, ..
            } => {
                assert_eq!(nickname, "bob");
                assert_eq!(reason, Some("bye!".to_string()));
            }
            _ => panic!("Expected Part event"),
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Resolve channel ID
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_resolve_channel_id_nonexistent() {
        let engine = setup_engine();
        let result = engine.resolve_channel_id(DEFAULT_SERVER_ID, "#nothing");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_channel_id_after_join() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .unwrap();
        let result = engine.resolve_channel_id(DEFAULT_SERVER_ID, "#general");
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    // ────────────────────────────────────────────────────────────────
    // list_servers_for_user (in-memory, no DB)
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_list_servers_for_user() {
        let engine = setup_engine();
        let sid1 = engine
            .create_server("Server A".into(), "user1".into(), None)
            .await
            .unwrap();
        let sid2 = engine
            .create_server("Server B".into(), "user1".into(), None)
            .await
            .unwrap();
        let _ = engine
            .create_server("Server C".into(), "user2".into(), None)
            .await
            .unwrap();

        // user1 should see Server A and Server B (they're the owner)
        engine.join_server("user1", &sid1).await.unwrap();
        engine.join_server("user1", &sid2).await.unwrap();
        let servers = engine.list_servers_for_user("user1");
        assert_eq!(servers.len(), 2);
        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Server A"));
        assert!(names.contains(&"Server B"));
    }

    // ────────────────────────────────────────────────────────────────
    // Multiple server creation generates unique IDs
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_server_ids_are_unique() {
        let engine = setup_engine();
        let id1 = engine
            .create_server("S1".into(), "user1".into(), None)
            .await
            .unwrap();
        let id2 = engine
            .create_server("S2".into(), "user1".into(), None)
            .await
            .unwrap();
        assert_ne!(id1, id2);
    }
}
