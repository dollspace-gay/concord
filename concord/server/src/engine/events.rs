use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a message.
pub type MessageId = Uuid;

/// Unique identifier for a connected session (one per connection, not per user).
pub type SessionId = Uuid;

/// Protocol-agnostic event that flows through the chat engine.
/// Both IRC and WebSocket adapters produce and consume these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// A message sent to a channel or as a DM.
    Message {
        id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_id: Option<String>,
        from: String,
        target: String,
        content: String,
        timestamp: DateTime<Utc>,
        #[serde(skip_serializing_if = "Option::is_none")]
        avatar_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to: Option<ReplyInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<AttachmentInfo>>,
    },

    /// A message was edited.
    MessageEdit {
        id: MessageId,
        server_id: String,
        channel: String,
        content: String,
        edited_at: DateTime<Utc>,
    },

    /// A message was deleted.
    MessageDelete {
        id: MessageId,
        server_id: String,
        channel: String,
    },

    /// Acknowledgment sent back to the sender with the server-generated message ID.
    /// The nonce matches the client-provided value so the frontend can update the optimistic message.
    MessageAck {
        id: MessageId,
        server_id: String,
        channel: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<String>,
    },

    /// A reaction was added to a message.
    ReactionAdd {
        message_id: MessageId,
        server_id: String,
        channel: String,
        user_id: String,
        nickname: String,
        emoji: String,
    },

    /// A reaction was removed from a message.
    ReactionRemove {
        message_id: MessageId,
        server_id: String,
        channel: String,
        user_id: String,
        nickname: String,
        emoji: String,
    },

    /// A user started typing in a channel.
    TypingStart {
        server_id: String,
        channel: String,
        nickname: String,
    },

    /// User joined a channel.
    Join {
        nickname: String,
        server_id: String,
        channel: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        avatar_url: Option<String>,
    },

    /// User left a channel.
    Part {
        nickname: String,
        server_id: String,
        channel: String,
        reason: Option<String>,
    },

    /// User disconnected from the server.
    Quit {
        nickname: String,
        reason: Option<String>,
    },

    /// Channel topic changed.
    TopicChange {
        server_id: String,
        channel: String,
        set_by: String,
        topic: String,
    },

    /// User changed their nickname.
    NickChange { old_nick: String, new_nick: String },

    /// Server notice directed at a specific session.
    ServerNotice { message: String },

    /// Channel member list (sent on join).
    Names {
        server_id: String,
        channel: String,
        members: Vec<MemberInfo>,
    },

    /// Current topic of a channel (sent on join).
    Topic {
        server_id: String,
        channel: String,
        topic: String,
    },

    /// Response to a channel list request.
    ChannelList {
        server_id: String,
        channels: Vec<ChannelInfo>,
    },

    /// Message history response.
    History {
        server_id: String,
        channel: String,
        messages: Vec<HistoryMessage>,
        has_more: bool,
    },

    /// List of servers the user belongs to.
    ServerList { servers: Vec<ServerInfo> },

    /// Unread message counts for channels in a server.
    UnreadCounts {
        server_id: String,
        counts: Vec<UnreadCount>,
    },

    /// Link embed previews were resolved for a message.
    MessageEmbed {
        message_id: MessageId,
        server_id: String,
        channel: String,
        embeds: Vec<EmbedInfo>,
    },

    /// List of roles in a server.
    RoleList {
        server_id: String,
        roles: Vec<RoleInfo>,
    },

    /// A role was created or updated.
    RoleUpdate { server_id: String, role: RoleInfo },

    /// A role was deleted.
    RoleDelete { server_id: String, role_id: String },

    /// A member's role assignments changed.
    MemberRoleUpdate {
        server_id: String,
        user_id: String,
        role_ids: Vec<String>,
    },

    /// List of categories in a server.
    CategoryList {
        server_id: String,
        categories: Vec<CategoryInfo>,
    },

    /// A category was created or updated.
    CategoryUpdate {
        server_id: String,
        category: CategoryInfo,
    },

    /// A category was deleted.
    CategoryDelete {
        server_id: String,
        category_id: String,
    },

    /// Channel positions/categories were reordered.
    ChannelReorder {
        server_id: String,
        channels: Vec<ChannelPositionInfo>,
    },

    /// Presence update for a user (broadcast to shared server members).
    PresenceUpdate {
        server_id: String,
        presence: PresenceInfo,
    },

    /// Bulk presence list for a server (sent on connect/join).
    PresenceList {
        server_id: String,
        presences: Vec<PresenceInfo>,
    },

    /// A user's profile was fetched or updated.
    UserProfile { profile: UserProfileInfo },

    /// A member's server nickname changed.
    ServerNicknameUpdate {
        server_id: String,
        user_id: String,
        nickname: Option<String>,
    },

    /// Notification settings response.
    NotificationSettings {
        server_id: String,
        settings: Vec<NotificationSettingInfo>,
    },

    /// Search results response.
    SearchResults {
        server_id: String,
        query: String,
        results: Vec<SearchResultMessage>,
        total_count: i64,
        offset: i64,
    },

    /// Message was pinned in a channel.
    MessagePin {
        server_id: String,
        channel: String,
        pin: PinnedMessageInfo,
    },

    /// Message was unpinned from a channel.
    MessageUnpin {
        server_id: String,
        channel: String,
        message_id: String,
    },

    /// List of all pinned messages in a channel.
    PinnedMessages {
        server_id: String,
        channel: String,
        pins: Vec<PinnedMessageInfo>,
    },

    /// A thread was created.
    ThreadCreate {
        server_id: String,
        parent_channel: String,
        thread: ThreadInfo,
    },

    /// A thread was archived or unarchived.
    ThreadUpdate {
        server_id: String,
        thread: ThreadInfo,
    },

    /// List of threads for a channel.
    ThreadList {
        server_id: String,
        channel: String,
        threads: Vec<ThreadInfo>,
    },

    /// Forum tags list.
    ForumTagList {
        server_id: String,
        channel: String,
        tags: Vec<ForumTagInfo>,
    },

    /// Forum tag created/updated.
    ForumTagUpdate {
        server_id: String,
        channel: String,
        tag: ForumTagInfo,
    },

    /// Forum tag deleted.
    ForumTagDelete {
        server_id: String,
        channel: String,
        tag_id: String,
    },

    /// Bookmarks list response.
    BookmarkList { bookmarks: Vec<BookmarkInfo> },

    /// Bookmark added.
    BookmarkAdd { bookmark: BookmarkInfo },

    /// Bookmark removed.
    BookmarkRemove { message_id: String },

    /// A member was kicked from the server.
    MemberKick {
        server_id: String,
        user_id: String,
        kicked_by: String,
        reason: Option<String>,
    },

    /// A member was banned from the server.
    MemberBan {
        server_id: String,
        user_id: String,
        banned_by: String,
        reason: Option<String>,
    },

    /// A ban was removed from the server.
    MemberUnban { server_id: String, user_id: String },

    /// A member was timed out.
    MemberTimeout {
        server_id: String,
        user_id: String,
        timeout_until: Option<String>,
    },

    /// Channel slow mode was updated.
    SlowModeUpdate {
        server_id: String,
        channel: String,
        seconds: i32,
    },

    /// Channel NSFW flag was updated.
    NsfwUpdate {
        server_id: String,
        channel: String,
        is_nsfw: bool,
    },

    /// Bulk messages were deleted.
    BulkMessageDelete {
        server_id: String,
        channel: String,
        message_ids: Vec<String>,
    },

    /// Audit log entries response.
    AuditLogEntries {
        server_id: String,
        entries: Vec<AuditLogEntry>,
    },

    /// Ban list response.
    BanList {
        server_id: String,
        bans: Vec<BanInfo>,
    },

    /// AutoMod rules list response.
    AutomodRuleList {
        server_id: String,
        rules: Vec<AutomodRuleInfo>,
    },

    /// AutoMod rule created/updated.
    AutomodRuleUpdate {
        server_id: String,
        rule: AutomodRuleInfo,
    },

    /// AutoMod rule deleted.
    AutomodRuleDelete { server_id: String, rule_id: String },

    // ── Phase 7: Community & Discovery ──
    /// Invite list response.
    InviteList {
        server_id: String,
        invites: Vec<InviteInfo>,
    },

    /// Invite created.
    InviteCreate {
        server_id: String,
        invite: InviteInfo,
    },

    /// Invite deleted.
    InviteDelete {
        server_id: String,
        invite_id: String,
    },

    /// Server events list.
    EventList {
        server_id: String,
        events: Vec<EventInfo>,
    },

    /// Event created or updated.
    EventUpdate { server_id: String, event: EventInfo },

    /// Event deleted.
    EventDelete { server_id: String, event_id: String },

    /// Event RSVP list.
    EventRsvpList {
        event_id: String,
        rsvps: Vec<RsvpInfo>,
    },

    /// Server community settings.
    ServerCommunity { community: ServerCommunityInfo },

    /// Discoverable servers list.
    DiscoverServers { servers: Vec<ServerCommunityInfo> },

    /// Channel follows list.
    ChannelFollowList {
        channel_id: String,
        follows: Vec<ChannelFollowInfo>,
    },

    /// Channel follow created.
    ChannelFollowCreate { follow: ChannelFollowInfo },

    /// Channel follow deleted.
    ChannelFollowDelete { follow_id: String },

    /// Server templates list.
    TemplateList {
        server_id: String,
        templates: Vec<TemplateInfo>,
    },

    /// Template created/updated.
    TemplateUpdate {
        server_id: String,
        template: TemplateInfo,
    },

    /// Template deleted.
    TemplateDelete {
        server_id: String,
        template_id: String,
    },

    // ── Phase 8: Integrations & Bots ──
    /// Webhook list response.
    WebhookList {
        server_id: String,
        webhooks: Vec<WebhookInfo>,
    },

    /// Webhook created or updated.
    WebhookUpdate {
        server_id: String,
        webhook: WebhookInfo,
    },

    /// Webhook deleted.
    WebhookDelete {
        server_id: String,
        webhook_id: String,
    },

    /// Slash commands list response.
    SlashCommandList {
        server_id: String,
        commands: Vec<SlashCommandInfo>,
    },

    /// Slash command created or updated.
    SlashCommandUpdate {
        server_id: String,
        command: SlashCommandInfo,
    },

    /// Slash command deleted.
    SlashCommandDelete {
        server_id: String,
        command_id: String,
    },

    /// An interaction was created (sent to the bot).
    InteractionCreate { interaction: InteractionInfo },

    /// An interaction response (sent back to the channel).
    InteractionResponse {
        interaction_id: String,
        server_id: String,
        channel: String,
        response: InteractionResponseData,
    },

    /// Bot tokens list response (sent only to the bot owner).
    BotTokenList {
        bot_user_id: String,
        tokens: Vec<BotTokenInfo>,
    },

    /// OAuth2 app list response.
    OAuth2AppList { apps: Vec<OAuth2AppInfo> },

    /// OAuth2 app created/updated.
    OAuth2AppUpdate { app: OAuth2AppInfo },

    /// Bluesky profile sync result.
    BlueskyProfileSync {
        user_id: String,
        bsky_handle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        avatar_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        banner_url: Option<String>,
        followers_count: i64,
        follows_count: i64,
    },

    /// Result of sharing a message to Bluesky.
    BlueskyShareResult {
        message_id: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        post_uri: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Per-server avatar updated.
    ServerAvatarUpdate {
        server_id: String,
        user_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        avatar_url: Option<String>,
    },

    /// Server configuration/limits info.
    ServerLimits {
        max_message_length: usize,
        max_file_size_mb: u64,
    },

    /// Error from the server.
    Error { code: String, message: String },
}

/// Info about a replied-to message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyInfo {
    pub id: String,
    pub from: String,
    pub content_preview: String,
}

/// Grouped reactions for a message in history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: usize,
    pub user_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub member_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Effective permission bitfield for the requesting user in this server.
    #[serde(default)]
    pub my_permissions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub topic: String,
    pub member_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub position: i32,
    pub is_private: bool,
    pub channel_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_parent_message_id: Option<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub id: MessageId,
    pub from: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reactions: Option<Vec<ReactionGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<AttachmentInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<EmbedInfo>>,
}

/// Metadata for a file attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub url: String,
}

/// Open Graph link embed preview metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedInfo {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnreadCount {
    pub channel_name: String,
    pub count: i64,
}

/// Role metadata sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub position: i32,
    pub permissions: i64,
    pub is_default: bool,
}

/// Channel category metadata sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub position: i32,
}

/// Minimal channel position info for reorder events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPositionInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub position: i32,
}

/// User presence info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub user_id: String,
    pub nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
}

/// Full user profile info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileInfo {
    pub user_id: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner_url: Option<String>,
    pub created_at: String,
}

/// Notification setting info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettingInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub level: String,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_until: Option<String>,
}

/// A search result message (same as HistoryMessage but with channel info).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultMessage {
    pub id: MessageId,
    pub from: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub channel_id: String,
    pub channel_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<DateTime<Utc>>,
}

/// Info about a pinned message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedMessageInfo {
    pub id: String,
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: String,
    // Denormalized message content for display
    pub from: String,
    pub content: String,
    pub timestamp: String,
}

/// Info about a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: String,
    pub name: String,
    pub channel_type: String, // "public_thread" | "private_thread"
    pub parent_message_id: Option<String>,
    pub archived: bool,
    pub auto_archive_minutes: i32,
    pub message_count: i64,
    pub created_at: String,
}

/// Forum tag info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumTagInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub moderated: bool,
    pub position: i32,
}

/// Bookmark info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkInfo {
    pub id: String,
    pub message_id: String,
    pub channel_id: String,
    pub from: String,
    pub content: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: String,
}

/// Audit log entry sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: String,
    pub actor_id: String,
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<String>,
    pub created_at: String,
}

/// Ban info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanInfo {
    pub id: String,
    pub user_id: String,
    pub banned_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: String,
}

/// AutoMod rule info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomodRuleInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub rule_type: String,
    pub config: String,
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_duration_seconds: Option<i32>,
}

/// Server invite info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteInfo {
    pub id: String,
    pub code: String,
    pub server_id: String,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<i32>,
    pub use_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub created_at: String,
}

/// Scheduled event info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub start_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub created_by: String,
    pub status: String,
    pub interested_count: i64,
    pub created_at: String,
}

/// RSVP info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsvpInfo {
    pub user_id: String,
    pub status: String,
}

/// Channel follow info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelFollowInfo {
    pub id: String,
    pub source_channel_id: String,
    pub target_channel_id: String,
    pub created_by: String,
}

/// Server template info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub server_id: String,
    pub created_by: String,
    pub use_count: i32,
    pub created_at: String,
}

/// Server community/discovery info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCommunityInfo {
    pub server_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_discoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

// ── Phase 8: Integrations & Bots ──

/// Webhook info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookInfo {
    pub id: String,
    pub server_id: String,
    pub channel_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub webhook_type: String,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

/// Slash command info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandInfo {
    pub id: String,
    pub bot_user_id: String,
    pub name: String,
    pub description: String,
    pub options: Vec<SlashCommandOption>,
}

/// A single option/parameter for a slash command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandOption {
    pub name: String,
    pub description: String,
    pub option_type: String, // "string", "integer", "boolean", "user", "channel", "role"
    #[serde(default)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<SlashCommandChoice>>,
}

/// A pre-defined choice for a slash command option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandChoice {
    pub name: String,
    pub value: String,
}

/// Interaction info sent to bots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionInfo {
    pub id: String,
    pub interaction_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_name: Option<String>,
    pub user_id: String,
    pub server_id: String,
    pub channel_id: String,
    pub data: serde_json::Value,
}

/// Bot's response to an interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<RichEmbedInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<MessageComponent>>,
    #[serde(default)]
    pub ephemeral: bool,
}

/// Rich embed format for bot messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichEmbedInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<EmbedField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// A field in a rich embed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

/// Footer for a rich embed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedFooter {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Author section for a rich embed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedAuthor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// A message component (button, select menu, or action row).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageComponent {
    ActionRow {
        components: Vec<MessageComponent>,
    },
    Button {
        custom_id: String,
        label: String,
        #[serde(default = "default_button_style")]
        style: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        emoji: Option<String>,
        #[serde(default)]
        disabled: bool,
    },
    SelectMenu {
        custom_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        options: Vec<SelectOption>,
        #[serde(default = "default_one")]
        min_values: i32,
        #[serde(default = "default_one")]
        max_values: i32,
    },
}

fn default_button_style() -> String {
    "primary".to_string()
}
fn default_one() -> i32 {
    1
}

/// An option in a select menu component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default)]
    pub default: bool,
}

/// Bot token info (without the actual hash).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotTokenInfo {
    pub id: String,
    pub name: String,
    pub scopes: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

/// OAuth2 application info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2AppInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub owner_id: String,
    pub redirect_uris: Vec<String>,
    pub scopes: String,
    pub is_public: bool,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ────────────────────────────────────────────────────────────────
    // ChatEvent serialization/deserialization round-trips
    // ────────────────────────────────────────────────────────────────

    fn roundtrip(event: &ChatEvent) -> ChatEvent {
        let json = serde_json::to_string(event).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    #[test]
    fn test_message_event_roundtrip() {
        let event = ChatEvent::Message {
            id: Uuid::new_v4(),
            server_id: Some("srv1".into()),
            from: "alice".into(),
            target: "#general".into(),
            content: "Hello, world!".into(),
            timestamp: Utc::now(),
            avatar_url: Some("https://example.com/avatar.png".into()),
            reply_to: Some(ReplyInfo {
                id: "msg-123".into(),
                from: "bob".into(),
                content_preview: "earlier message".into(),
            }),
            attachments: Some(vec![AttachmentInfo {
                id: "att-1".into(),
                filename: "file.txt".into(),
                content_type: "text/plain".into(),
                file_size: 1234,
                url: "https://example.com/file.txt".into(),
            }]),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::Message {
                from,
                target,
                content,
                server_id,
                reply_to,
                attachments,
                ..
            } => {
                assert_eq!(from, "alice");
                assert_eq!(target, "#general");
                assert_eq!(content, "Hello, world!");
                assert_eq!(server_id, Some("srv1".into()));
                assert!(reply_to.is_some());
                assert_eq!(reply_to.unwrap().from, "bob");
                assert_eq!(attachments.unwrap().len(), 1);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_message_event_minimal() {
        let event = ChatEvent::Message {
            id: Uuid::new_v4(),
            server_id: None,
            from: "alice".into(),
            target: "bob".into(),
            content: "DM".into(),
            timestamp: Utc::now(),
            avatar_url: None,
            reply_to: None,
            attachments: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        // Optional None fields should be skipped
        assert!(!json.contains("server_id"));
        assert!(!json.contains("avatar_url"));
        assert!(!json.contains("reply_to"));
        assert!(!json.contains("attachments"));
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::Message { from, target, .. } => {
                assert_eq!(from, "alice");
                assert_eq!(target, "bob");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_message_edit_event_roundtrip() {
        let event = ChatEvent::MessageEdit {
            id: Uuid::new_v4(),
            server_id: "srv1".into(),
            channel: "#general".into(),
            content: "edited content".into(),
            edited_at: Utc::now(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::MessageEdit {
                content, channel, ..
            } => {
                assert_eq!(content, "edited content");
                assert_eq!(channel, "#general");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_message_delete_event_roundtrip() {
        let event = ChatEvent::MessageDelete {
            id: Uuid::new_v4(),
            server_id: "srv1".into(),
            channel: "#general".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::MessageDelete { channel, .. } => {
                assert_eq!(channel, "#general");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_join_event_roundtrip() {
        let event = ChatEvent::Join {
            nickname: "alice".into(),
            server_id: "srv1".into(),
            channel: "#general".into(),
            avatar_url: Some("https://example.com/avatar.png".into()),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::Join {
                nickname,
                avatar_url,
                ..
            } => {
                assert_eq!(nickname, "alice");
                assert!(avatar_url.is_some());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_part_event_roundtrip() {
        let event = ChatEvent::Part {
            nickname: "alice".into(),
            server_id: "srv1".into(),
            channel: "#general".into(),
            reason: Some("goodbye".into()),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::Part {
                nickname, reason, ..
            } => {
                assert_eq!(nickname, "alice");
                assert_eq!(reason, Some("goodbye".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_quit_event_roundtrip() {
        let event = ChatEvent::Quit {
            nickname: "alice".into(),
            reason: None,
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::Quit { nickname, reason } => {
                assert_eq!(nickname, "alice");
                assert!(reason.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_topic_change_event_roundtrip() {
        let event = ChatEvent::TopicChange {
            server_id: "srv1".into(),
            channel: "#general".into(),
            set_by: "alice".into(),
            topic: "New topic".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::TopicChange { topic, set_by, .. } => {
                assert_eq!(topic, "New topic");
                assert_eq!(set_by, "alice");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_nick_change_event_roundtrip() {
        let event = ChatEvent::NickChange {
            old_nick: "alice".into(),
            new_nick: "alice2".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::NickChange { old_nick, new_nick } => {
                assert_eq!(old_nick, "alice");
                assert_eq!(new_nick, "alice2");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_server_notice_event_roundtrip() {
        let event = ChatEvent::ServerNotice {
            message: "Welcome!".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::ServerNotice { message } => {
                assert_eq!(message, "Welcome!");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_error_event_roundtrip() {
        let event = ChatEvent::Error {
            code: "FORBIDDEN".into(),
            message: "No permission".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::Error { code, message } => {
                assert_eq!(code, "FORBIDDEN");
                assert_eq!(message, "No permission");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_reaction_add_event_roundtrip() {
        let event = ChatEvent::ReactionAdd {
            message_id: Uuid::new_v4(),
            server_id: "srv1".into(),
            channel: "#general".into(),
            user_id: "user1".into(),
            nickname: "alice".into(),
            emoji: "\u{1F44D}".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::ReactionAdd {
                emoji, nickname, ..
            } => {
                assert_eq!(emoji, "\u{1F44D}");
                assert_eq!(nickname, "alice");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_typing_start_event_roundtrip() {
        let event = ChatEvent::TypingStart {
            server_id: "srv1".into(),
            channel: "#general".into(),
            nickname: "alice".into(),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::TypingStart { nickname, .. } => {
                assert_eq!(nickname, "alice");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_member_kick_event_roundtrip() {
        let event = ChatEvent::MemberKick {
            server_id: "srv1".into(),
            user_id: "user1".into(),
            kicked_by: "admin1".into(),
            reason: Some("violated rules".into()),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::MemberKick {
                user_id,
                kicked_by,
                reason,
                ..
            } => {
                assert_eq!(user_id, "user1");
                assert_eq!(kicked_by, "admin1");
                assert_eq!(reason, Some("violated rules".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_member_ban_event_roundtrip() {
        let event = ChatEvent::MemberBan {
            server_id: "srv1".into(),
            user_id: "user1".into(),
            banned_by: "admin1".into(),
            reason: None,
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::MemberBan {
                user_id, reason, ..
            } => {
                assert_eq!(user_id, "user1");
                assert!(reason.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_member_timeout_event_roundtrip() {
        let event = ChatEvent::MemberTimeout {
            server_id: "srv1".into(),
            user_id: "user1".into(),
            timeout_until: Some("2026-03-01T00:00:00Z".into()),
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::MemberTimeout {
                user_id,
                timeout_until,
                ..
            } => {
                assert_eq!(user_id, "user1");
                assert_eq!(timeout_until, Some("2026-03-01T00:00:00Z".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_slow_mode_update_event_roundtrip() {
        let event = ChatEvent::SlowModeUpdate {
            server_id: "srv1".into(),
            channel: "#general".into(),
            seconds: 10,
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::SlowModeUpdate { seconds, .. } => {
                assert_eq!(seconds, 10);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_bulk_message_delete_event_roundtrip() {
        let event = ChatEvent::BulkMessageDelete {
            server_id: "srv1".into(),
            channel: "#general".into(),
            message_ids: vec!["msg1".into(), "msg2".into(), "msg3".into()],
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::BulkMessageDelete { message_ids, .. } => {
                assert_eq!(message_ids.len(), 3);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_webhook_list_event_roundtrip() {
        let event = ChatEvent::WebhookList {
            server_id: "srv1".into(),
            webhooks: vec![WebhookInfo {
                id: "wh1".into(),
                server_id: "srv1".into(),
                channel_id: "ch1".into(),
                name: "My Webhook".into(),
                avatar_url: None,
                webhook_type: "incoming".into(),
                token: "token123".into(),
                url: None,
                created_by: "user1".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            }],
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::WebhookList { webhooks, .. } => {
                assert_eq!(webhooks.len(), 1);
                assert_eq!(webhooks[0].name, "My Webhook");
                assert_eq!(webhooks[0].webhook_type, "incoming");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_slash_command_list_event_roundtrip() {
        let event = ChatEvent::SlashCommandList {
            server_id: "srv1".into(),
            commands: vec![SlashCommandInfo {
                id: "cmd1".into(),
                bot_user_id: "bot1".into(),
                name: "ping".into(),
                description: "Pings the bot".into(),
                options: vec![SlashCommandOption {
                    name: "target".into(),
                    description: "Who to ping".into(),
                    option_type: "user".into(),
                    required: true,
                    choices: None,
                }],
            }],
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::SlashCommandList { commands, .. } => {
                assert_eq!(commands.len(), 1);
                assert_eq!(commands[0].name, "ping");
                assert_eq!(commands[0].options.len(), 1);
                assert!(commands[0].options[0].required);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_interaction_create_event_roundtrip() {
        let event = ChatEvent::InteractionCreate {
            interaction: InteractionInfo {
                id: "int1".into(),
                interaction_type: "slash_command".into(),
                command_name: Some("ping".into()),
                user_id: "user1".into(),
                server_id: "srv1".into(),
                channel_id: "ch1".into(),
                data: serde_json::json!({"target": "user2"}),
            },
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::InteractionCreate { interaction } => {
                assert_eq!(interaction.id, "int1");
                assert_eq!(interaction.command_name, Some("ping".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_oauth2_app_list_event_roundtrip() {
        let event = ChatEvent::OAuth2AppList {
            apps: vec![OAuth2AppInfo {
                id: "app1".into(),
                name: "My App".into(),
                description: "Test app".into(),
                icon_url: None,
                owner_id: "user1".into(),
                redirect_uris: vec!["https://example.com/callback".into()],
                scopes: "identify".into(),
                is_public: true,
                created_at: "2026-01-01T00:00:00Z".into(),
            }],
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::OAuth2AppList { apps } => {
                assert_eq!(apps.len(), 1);
                assert_eq!(apps[0].name, "My App");
                assert!(apps[0].is_public);
                assert_eq!(apps[0].redirect_uris.len(), 1);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_invite_create_event_roundtrip() {
        let event = ChatEvent::InviteCreate {
            server_id: "srv1".into(),
            invite: InviteInfo {
                id: "inv1".into(),
                code: "abc12345".into(),
                server_id: "srv1".into(),
                created_by: "user1".into(),
                max_uses: Some(10),
                use_count: 0,
                expires_at: Some("2026-12-31T23:59:59Z".into()),
                channel_id: None,
                created_at: "2026-01-01T00:00:00Z".into(),
            },
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::InviteCreate { invite, .. } => {
                assert_eq!(invite.code, "abc12345");
                assert_eq!(invite.max_uses, Some(10));
                assert_eq!(invite.use_count, 0);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_event_update_event_roundtrip() {
        let event = ChatEvent::EventUpdate {
            server_id: "srv1".into(),
            event: EventInfo {
                id: "ev1".into(),
                server_id: "srv1".into(),
                name: "Game Night".into(),
                description: Some("Weekly game night".into()),
                channel_id: None,
                start_time: "2026-03-01T20:00:00Z".into(),
                end_time: Some("2026-03-01T23:00:00Z".into()),
                image_url: None,
                created_by: "user1".into(),
                status: "scheduled".into(),
                interested_count: 5,
                created_at: "2026-01-01T00:00:00Z".into(),
            },
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::EventUpdate { event: ei, .. } => {
                assert_eq!(ei.name, "Game Night");
                assert_eq!(ei.status, "scheduled");
                assert_eq!(ei.interested_count, 5);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_thread_create_event_roundtrip() {
        let event = ChatEvent::ThreadCreate {
            server_id: "srv1".into(),
            parent_channel: "#general".into(),
            thread: ThreadInfo {
                id: "thread1".into(),
                name: "#my-thread".into(),
                channel_type: "public_thread".into(),
                parent_message_id: Some("msg1".into()),
                archived: false,
                auto_archive_minutes: 1440,
                message_count: 0,
                created_at: "2026-01-01T00:00:00Z".into(),
            },
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::ThreadCreate { thread, .. } => {
                assert_eq!(thread.name, "#my-thread");
                assert_eq!(thread.channel_type, "public_thread");
                assert!(!thread.archived);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_pinned_messages_event_roundtrip() {
        let event = ChatEvent::PinnedMessages {
            server_id: "srv1".into(),
            channel: "#general".into(),
            pins: vec![PinnedMessageInfo {
                id: "pin1".into(),
                message_id: "msg1".into(),
                channel_id: "ch1".into(),
                pinned_by: "user1".into(),
                pinned_at: "2026-01-01T00:00:00Z".into(),
                from: "alice".into(),
                content: "Important message".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
            }],
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::PinnedMessages { pins, .. } => {
                assert_eq!(pins.len(), 1);
                assert_eq!(pins[0].content, "Important message");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_presence_update_event_roundtrip() {
        let event = ChatEvent::PresenceUpdate {
            server_id: "srv1".into(),
            presence: PresenceInfo {
                user_id: "user1".into(),
                nickname: "alice".into(),
                avatar_url: None,
                status: "online".into(),
                custom_status: Some("Coding!".into()),
                status_emoji: Some("\u{1F4BB}".into()),
            },
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::PresenceUpdate { presence, .. } => {
                assert_eq!(presence.status, "online");
                assert_eq!(presence.custom_status, Some("Coding!".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_discover_servers_event_roundtrip() {
        let event = ChatEvent::DiscoverServers {
            servers: vec![ServerCommunityInfo {
                server_id: "srv1".into(),
                description: Some("A fun server".into()),
                is_discoverable: true,
                welcome_message: Some("Welcome!".into()),
                rules_text: None,
                category: Some("gaming".into()),
            }],
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::DiscoverServers { servers } => {
                assert_eq!(servers.len(), 1);
                assert!(servers[0].is_discoverable);
                assert_eq!(servers[0].category, Some("gaming".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Serde tag correctness
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_event_json_has_type_tag() {
        let event = ChatEvent::Message {
            id: Uuid::new_v4(),
            server_id: None,
            from: "a".into(),
            target: "b".into(),
            content: "c".into(),
            timestamp: Utc::now(),
            avatar_url: None,
            reply_to: None,
            attachments: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"message""#));
    }

    #[test]
    fn test_event_type_tags_are_snake_case() {
        let events: Vec<(ChatEvent, &str)> = vec![
            (
                ChatEvent::MessageEdit {
                    id: Uuid::new_v4(),
                    server_id: "s".into(),
                    channel: "c".into(),
                    content: "x".into(),
                    edited_at: Utc::now(),
                },
                "message_edit",
            ),
            (
                ChatEvent::MessageDelete {
                    id: Uuid::new_v4(),
                    server_id: "s".into(),
                    channel: "c".into(),
                },
                "message_delete",
            ),
            (
                ChatEvent::TopicChange {
                    server_id: "s".into(),
                    channel: "c".into(),
                    set_by: "a".into(),
                    topic: "t".into(),
                },
                "topic_change",
            ),
            (
                ChatEvent::NickChange {
                    old_nick: "a".into(),
                    new_nick: "b".into(),
                },
                "nick_change",
            ),
            (
                ChatEvent::ServerNotice {
                    message: "hi".into(),
                },
                "server_notice",
            ),
            (
                ChatEvent::TypingStart {
                    server_id: "s".into(),
                    channel: "c".into(),
                    nickname: "a".into(),
                },
                "typing_start",
            ),
            (
                ChatEvent::MemberKick {
                    server_id: "s".into(),
                    user_id: "u".into(),
                    kicked_by: "a".into(),
                    reason: None,
                },
                "member_kick",
            ),
            (
                ChatEvent::BulkMessageDelete {
                    server_id: "s".into(),
                    channel: "c".into(),
                    message_ids: vec![],
                },
                "bulk_message_delete",
            ),
        ];

        for (event, expected_type) in events {
            let json = serde_json::to_string(&event).unwrap();
            let expected = format!(r#""type":"{}""#, expected_type);
            assert!(
                json.contains(&expected),
                "Event type tag should be '{}', got json: {}",
                expected_type,
                json
            );
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Struct field completeness / Debug
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_all_info_structs_implement_debug() {
        // This test verifies Debug is implemented by using format!
        let _ = format!(
            "{:?}",
            ReplyInfo {
                id: "1".into(),
                from: "a".into(),
                content_preview: "b".into(),
            }
        );
        let _ = format!(
            "{:?}",
            ReactionGroup {
                emoji: "e".into(),
                count: 1,
                user_ids: vec![],
            }
        );
        let _ = format!(
            "{:?}",
            ServerInfo {
                id: "1".into(),
                name: "n".into(),
                icon_url: None,
                member_count: 0,
                role: None,
                my_permissions: 0,
            }
        );
        let _ = format!(
            "{:?}",
            ChannelInfo {
                id: "1".into(),
                server_id: "s".into(),
                name: "n".into(),
                topic: "t".into(),
                member_count: 0,
                category_id: None,
                position: 0,
                is_private: false,
                channel_type: "text".into(),
                thread_parent_message_id: None,
                archived: false,
            }
        );
        let _ = format!(
            "{:?}",
            MemberInfo {
                nickname: "n".into(),
                avatar_url: None,
                status: None,
                custom_status: None,
                status_emoji: None,
                user_id: None,
                server_avatar_url: None,
            }
        );
        let _ = format!(
            "{:?}",
            EmbedInfo {
                url: "u".into(),
                title: None,
                description: None,
                image_url: None,
                site_name: None,
            }
        );
        let _ = format!(
            "{:?}",
            WebhookInfo {
                id: "1".into(),
                server_id: "s".into(),
                channel_id: "c".into(),
                name: "n".into(),
                avatar_url: None,
                webhook_type: "incoming".into(),
                token: "t".into(),
                url: None,
                created_by: "u".into(),
                created_at: "d".into(),
            }
        );
        let _ = format!(
            "{:?}",
            BotTokenInfo {
                id: "1".into(),
                name: "n".into(),
                scopes: "bot".into(),
                created_at: "d".into(),
                last_used: None,
            }
        );
        let _ = format!(
            "{:?}",
            OAuth2AppInfo {
                id: "1".into(),
                name: "n".into(),
                description: "d".into(),
                icon_url: None,
                owner_id: "o".into(),
                redirect_uris: vec![],
                scopes: "identify".into(),
                is_public: false,
                created_at: "d".into(),
            }
        );
    }

    #[test]
    fn test_all_info_structs_implement_clone() {
        let ri = ReplyInfo {
            id: "1".into(),
            from: "a".into(),
            content_preview: "b".into(),
        };
        let cloned = ri.clone();
        assert_eq!(cloned.id, "1");

        let wi = WebhookInfo {
            id: "1".into(),
            server_id: "s".into(),
            channel_id: "c".into(),
            name: "n".into(),
            avatar_url: None,
            webhook_type: "incoming".into(),
            token: "t".into(),
            url: None,
            created_by: "u".into(),
            created_at: "d".into(),
        };
        let cloned = wi.clone();
        assert_eq!(cloned.name, "n");
    }

    // ────────────────────────────────────────────────────────────────
    // MessageComponent serialization
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_button_component_roundtrip() {
        let component = MessageComponent::Button {
            custom_id: "btn1".into(),
            label: "Click me".into(),
            style: "primary".into(),
            emoji: Some("\u{1F44D}".into()),
            disabled: false,
        };
        let json = serde_json::to_string(&component).unwrap();
        let restored: MessageComponent = serde_json::from_str(&json).unwrap();
        match restored {
            MessageComponent::Button {
                custom_id, label, ..
            } => {
                assert_eq!(custom_id, "btn1");
                assert_eq!(label, "Click me");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_select_menu_component_roundtrip() {
        let component = MessageComponent::SelectMenu {
            custom_id: "select1".into(),
            placeholder: Some("Choose...".into()),
            options: vec![SelectOption {
                label: "Option A".into(),
                value: "a".into(),
                description: Some("First option".into()),
                emoji: None,
                default: true,
            }],
            min_values: 1,
            max_values: 3,
        };
        let json = serde_json::to_string(&component).unwrap();
        let restored: MessageComponent = serde_json::from_str(&json).unwrap();
        match restored {
            MessageComponent::SelectMenu {
                options,
                max_values,
                ..
            } => {
                assert_eq!(options.len(), 1);
                assert!(options[0].default);
                assert_eq!(max_values, 3);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_action_row_component_roundtrip() {
        let component = MessageComponent::ActionRow {
            components: vec![
                MessageComponent::Button {
                    custom_id: "btn1".into(),
                    label: "Yes".into(),
                    style: "primary".into(),
                    emoji: None,
                    disabled: false,
                },
                MessageComponent::Button {
                    custom_id: "btn2".into(),
                    label: "No".into(),
                    style: "danger".into(),
                    emoji: None,
                    disabled: false,
                },
            ],
        };
        let json = serde_json::to_string(&component).unwrap();
        let restored: MessageComponent = serde_json::from_str(&json).unwrap();
        match restored {
            MessageComponent::ActionRow { components } => {
                assert_eq!(components.len(), 2);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_interaction_response_data_roundtrip() {
        let resp = InteractionResponseData {
            content: Some("Hello!".into()),
            embeds: Some(vec![RichEmbedInfo {
                title: Some("Title".into()),
                description: Some("Desc".into()),
                url: None,
                color: Some("#FF0000".into()),
                fields: Some(vec![EmbedField {
                    name: "Field 1".into(),
                    value: "Value 1".into(),
                    inline: true,
                }]),
                footer: Some(EmbedFooter {
                    text: "Footer text".into(),
                    icon_url: None,
                }),
                image_url: None,
                thumbnail_url: None,
                author: Some(EmbedAuthor {
                    name: "Author".into(),
                    url: None,
                    icon_url: None,
                }),
                timestamp: None,
            }]),
            components: None,
            ephemeral: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let restored: InteractionResponseData = serde_json::from_str(&json).unwrap();
        assert!(restored.ephemeral);
        assert_eq!(restored.content, Some("Hello!".into()));
        let embeds = restored.embeds.unwrap();
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].title, Some("Title".into()));
        let fields = embeds[0].fields.as_ref().unwrap();
        assert_eq!(fields.len(), 1);
        assert!(fields[0].inline);
    }

    #[test]
    fn test_bluesky_profile_sync_roundtrip() {
        let event = ChatEvent::BlueskyProfileSync {
            user_id: "user1".into(),
            bsky_handle: "alice.bsky.social".into(),
            display_name: Some("Alice".into()),
            description: Some("Hello world".into()),
            avatar_url: Some("https://cdn.bsky.app/avatar.jpg".into()),
            banner_url: None,
            followers_count: 150,
            follows_count: 42,
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::BlueskyProfileSync {
                user_id,
                bsky_handle,
                display_name,
                followers_count,
                ..
            } => {
                assert_eq!(user_id, "user1");
                assert_eq!(bsky_handle, "alice.bsky.social");
                assert_eq!(display_name.as_deref(), Some("Alice"));
                assert_eq!(followers_count, 150);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_bluesky_share_result_roundtrip() {
        let event = ChatEvent::BlueskyShareResult {
            message_id: "msg1".into(),
            success: true,
            post_uri: Some("at://did:plc:abc/app.bsky.feed.post/xyz".into()),
            error: None,
        };
        let restored = roundtrip(&event);
        match restored {
            ChatEvent::BlueskyShareResult {
                message_id,
                success,
                post_uri,
                error,
            } => {
                assert_eq!(message_id, "msg1");
                assert!(success);
                assert!(post_uri.is_some());
                assert!(error.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }
}
