use super::authorization::ChannelAction;
use super::channel::ChannelState;
use super::events::{
    AuditLogEntry, AutomodRuleInfo, BanInfo, BookmarkInfo, BotTokenInfo, CategoryInfo,
    ChannelFollowInfo, ChannelInfo, ChannelPositionInfo, ChatEvent, ConnectionId,
    DirectConversationInfo, EventInfo, HistoryMessage, InteractionInfo, InteractionResponseData,
    InviteInfo, MemberInfo, MemberRoleInfo, OAuth2AppInfo, PinnedMessageInfo, ReactionGroup,
    ReplyInfo, RoleInfo, RsvpInfo, ServerCommunityInfo, ServerInfo, SlashCommandInfo,
    SlashCommandOption, TemplateInfo, ThreadInfo, WebhookInfo,
};
use super::ids::{ChannelId, ServerId};
use super::moderation::ModerationError;
use super::permissions::{
    DEFAULT_ADMIN, DEFAULT_EVERYONE, DEFAULT_MODERATOR, Permissions, ServerRole,
};
use super::rate_limiter::RateLimiter;
use super::server::ServerState;
use super::user_session::{Protocol, UserSession};
use chrono::Utc;
use dashmap::DashMap;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// The central hub that manages all chat state. Protocol-agnostic —
/// both IRC and WebSocket adapters call into this.
pub struct ChatEngine {
    /// All currently connected sessions, keyed by session ID.
    sessions: DashMap<ConnectionId, Arc<UserSession>>,
    user_connections: DashMap<String, std::collections::HashSet<ConnectionId>>,
    /// All servers (guilds), keyed by server ID.
    servers: DashMap<String, ServerState>,
    /// All channels, keyed by channel UUID.
    channels: DashMap<String, ChannelState>,
    /// Index: (server_id, channel_name) -> channel_id for name-based lookups.
    channel_name_index: DashMap<(String, String), String>,
    server_alias_index: DashMap<String, String>,
    server_aliases: DashMap<String, String>,
    /// Reverse lookup: nickname -> session ID (for DMs and WHOIS).
    nick_to_session: DashMap<String, ConnectionId>,
    /// Durable authenticated principal bound to each production connection.
    authenticated_actors: DashMap<ConnectionId, crate::auth::authority::Actor>,
    credential_connections:
        DashMap<crate::auth::authority::CredentialId, std::collections::HashSet<ConnectionId>>,
    /// Optional database pool. When present, messages and channels are persisted.
    db: Option<SqlitePool>,
    auth: OnceLock<crate::auth::authority::AuthService>,
    messaging: OnceLock<super::messaging::MessagingService>,
    replay: OnceLock<super::replay::ReplayService>,
    search_token_secret: String,
    integration_vault: OnceLock<Arc<crate::secrets::SecretVault>>,
    write_admission: Option<super::write_admission::WriteAdmission>,
    /// Per-user message rate limiter (burst of 10, refill 1 per second).
    message_limiter: RateLimiter,
    /// Maximum message content length (configurable, default 4000).
    max_message_length: usize,
    /// Maximum file upload size in megabytes (configurable, default 100).
    max_file_size_mb: u64,
    /// In-memory slow mode tracker: (user_id, channel_id) -> last message Instant.
    /// Prevents concurrent requests from bypassing the DB-based cooldown check.
    slowmode_last_sent: DashMap<(String, String), Instant>,
}

mod account_projection;
mod announcements;
mod automod;
mod bookmarks;
mod bootstrap;
mod bots;
mod broadcast;
mod categories;
mod channel_membership;
mod channel_projection;
mod community;
mod component_invocation;
mod connections;
mod construction;
mod delivery_authority;
mod dispatch;
mod forum_tags;
mod history;
mod incoming_webhooks;
mod interaction_response;
mod invites;
mod irc_identity;
mod legacy_mutations;
#[cfg(test)]
mod legacy_send;
mod maintenance;
mod managed_media;
mod message_commands;
mod message_mutations;
mod moderation;
mod notifications;
mod oauth_apps;
mod pins;
mod presence;
mod profiles;
mod publication;
mod read_state;
mod role_projection;
mod roles;
mod scheduled_events;
mod search;
mod search_types;
mod server_lifecycle;
mod server_membership;
mod service_access;
mod slash_commands;
mod slash_invocation;
mod synchronization;
mod templates;
mod thread_projection;
mod threads;
mod webhooks;
use search_types::{SearchContinuationClaims, SearchQueryPlan};
pub use search_types::{SearchError, SearchMessagesRequest, SearchResultsPage};
mod requests;
use requests::PresenceProjectionRow;
pub use requests::{
    AtprotoPublicationError, CreateAutomodRuleRequest, CreateServerEventRequest, DEFAULT_SERVER_ID,
    Synchronization, UpdateAutomodRuleRequest, UpdateNotificationSettingsParams,
};
mod search_query;
use search_query::{parse_search_query, search_fingerprint};
mod interaction_validation;
use interaction_validation::{
    find_message_component, validate_rich_interaction_response, validate_slash_command_arguments,
    validate_slash_command_options,
};
mod projections;
use projections::{
    category_row_to_info, channel_conversation_id, group_member_roles, moderation_dependency,
    moderation_unauthenticated, moderation_unavailable, normalize_channel_name,
    parse_persisted_timestamp, referenced_channel_id, referenced_server_id, role_row_to_info,
    server_member_display_identity, stable_irc_alias, webhook_row_to_info,
};

#[cfg(test)]
mod tests;
