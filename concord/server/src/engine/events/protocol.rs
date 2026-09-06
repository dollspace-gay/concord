use super::{
    AttachmentInfo, AuditLogEntry, AutomodRuleInfo, BanInfo, BookmarkInfo, BotAccountInfo,
    BotTokenInfo, CategoryInfo, ChannelFollowInfo, ChannelInfo, ChannelPermissionOverrideInfo,
    ChannelPositionInfo, DateTime, Deserialize, DirectConversationInfo, EmbedInfo, EventInfo,
    ForumTagInfo, HistoryMessage, InteractionInfo, InteractionResponseData, InviteInfo, JsonSchema,
    MemberInfo, MemberRoleInfo, MessageId, NotificationSettingInfo, OAuth2AppInfo,
    PinnedMessageInfo, PresenceInfo, ReplyInfo, RoleInfo, RsvpInfo, SearchResultMessage, Serialize,
    ServerCommunityInfo, ServerInfo, SlashCommandInfo, TemplateInfo, ThreadInfo, UnreadCount,
    UserProfileInfo, Utc, WebhookInfo,
};

/// Protocol-agnostic event that flows through the chat engine.
/// Both IRC and WebSocket adapters produce and consume these.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// Protocol-v2 synchronization snapshot at an opaque durable cursor.
    SyncSnapshot {
        request_id: String,
        snapshot: crate::engine::replay::SyncSnapshot,
    },

    /// Protocol-v2 durable replay batch.
    ReplayBatch {
        request_id: String,
        batch: crate::engine::replay::ReplayBatch,
    },

    /// Current authorized projection of a durable event descriptor.
    DurableEvent {
        event: Box<crate::engine::replay::DurableEventProjection>,
    },

    /// The supplied cursor cannot safely resume and the client must replace cached state.
    ResyncRequired {
        request_id: String,
        reason: crate::engine::replay::ResyncReason,
    },

    /// Correlated stable command failure for protocol-v2 clients.
    CommandError {
        request_id: String,
        code: String,
        message: String,
        retryable: bool,
    },

    /// Correlated acceptance of a non-durable lifecycle command.
    ///
    /// Clients add `request_id` to the command object. Serde deliberately
    /// ignores that field on older command variants, while the WebSocket
    /// adapter retains it and emits this result only after the command has
    /// completed successfully.
    LifecycleCommandSucceeded { request_id: String },

    /// Durable correlated result for edit/delete/reaction/read commands.
    CommandCommitted {
        receipt: crate::engine::messaging::CommandReceipt,
    },

    /// A message sent to a channel or as a DM.
    Message {
        id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_id: Option<String>,
        /// Canonical conversation identifier, present for direct messages.
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
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
        conversation_id: Option<String>,
        request_id: String,
        client_message_id: String,
        /// Decimal string to preserve the full SQLite integer range in JavaScript.
        sequence: String,
        persisted_at: String,
        replayed: bool,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        user_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_avatar_url: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        role_ids: Vec<String>,
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
        version: i64,
        roles: Vec<RoleInfo>,
        /// Present for an authoritative bootstrap; absent for a metadata-only mutation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member_roles: Option<Vec<MemberRoleInfo>>,
    },

    /// A role was created or updated.
    RoleUpdate { server_id: String, role: RoleInfo },

    /// A role was deleted.
    RoleDelete { server_id: String, role_id: String },

    /// A member's role assignments changed.
    MemberRoleUpdate {
        server_id: String,
        version: i64,
        user_id: String,
        role_ids: Vec<String>,
    },

    /// Authoritative permission rules for one channel.
    ChannelPermissionOverrideList {
        server_id: String,
        channel_id: String,
        overrides: Vec<ChannelPermissionOverrideInfo>,
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

    /// The authenticated user's durable preference and current projected state.
    OwnPresence {
        requested_status: String,
        effective_status: String,
        custom_status: Option<String>,
        status_emoji: Option<String>,
    },

    /// A user's profile was fetched or updated.
    UserProfile { profile: UserProfileInfo },

    /// A member's server nickname changed.
    ServerNicknameUpdate {
        server_id: String,
        user_id: String,
        nickname: Option<String>,
        display_name: String,
        server_avatar_url: Option<String>,
    },

    /// Notification settings response.
    NotificationSettings {
        server_id: String,
        settings: Vec<NotificationSettingInfo>,
    },

    /// Search results response.
    SearchResults {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        server_id: String,
        query: String,
        results: Vec<SearchResultMessage>,
        total_count: i64,
        offset: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_continuation: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        restarted: bool,
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

    /// Complete tag selection for a thread.
    ThreadTagUpdate {
        server_id: String,
        thread_id: String,
        version: i64,
        tag_ids: Vec<String>,
    },

    /// Bookmarks list response.
    BookmarkList { bookmarks: Vec<BookmarkInfo> },

    /// Bookmark added.
    BookmarkAdd { bookmark: BookmarkInfo },

    /// Bookmark removed.
    BookmarkRemove { message_id: String },

    /// Actor-scoped direct conversation navigation state.
    DirectConversationList {
        conversations: Vec<DirectConversationInfo>,
    },

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

    /// Result of an explicit announcement publication request.
    AnnouncementPublished {
        source_message_id: String,
        published_count: usize,
    },

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

    /// A new server created atomically from a versioned template.
    TemplateInstantiated {
        template_id: String,
        server_id: String,
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
    /// Correlated acceptance of a slash command or message component invocation.
    InteractionInvoked { request_id: String },

    /// Bot tokens list response (sent only to the bot owner).
    BotTokenList {
        bot_user_id: String,
        tokens: Vec<BotTokenInfo>,
    },
    /// Bot accounts controlled by the authenticated owner.
    BotAccountList { bots: Vec<BotAccountInfo> },
    /// A newly issued secret. The raw token is emitted exactly once.
    BotCredentialCreated {
        bot_user_id: String,
        token: String,
        credential: BotTokenInfo,
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
