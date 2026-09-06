use super::{DEFAULT_SERVER_ID, Deserialize, JsonSchema, Serialize};

pub(super) fn default_oauth_client_type() -> String {
    "confidential".to_owned()
}

/// Client-to-server WebSocket message types.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientMessage {
    /// Explicit correlation envelope for non-durable lifecycle commands.
    LifecycleCommand {
        request_id: String,
        command: Box<ClientMessage>,
    },
    Sync {
        request_id: String,
        protocol_version: u32,
        subscriptions: Vec<String>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    SendMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        content: String,
        #[serde(default)]
        content_format: crate::engine::messaging::ContentFormat,
        reply_to: Option<String>,
        attachment_ids: Option<Vec<String>>,
        #[serde(default)]
        mentions: Vec<crate::engine::messaging::MessageMention>,
        nonce: Option<String>,
    },
    SendDirectMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        recipient: String,
        content: String,
        #[serde(default)]
        content_format: crate::engine::messaging::ContentFormat,
        reply_to: Option<String>,
        attachment_ids: Option<Vec<String>>,
        nonce: Option<String>,
    },
    ListDirectConversations,
    JoinChannel {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    PartChannel {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        reason: Option<String>,
    },
    SetTopic {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        topic: String,
    },
    FetchHistory {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        before: Option<String>,
        limit: Option<i64>,
    },
    ListChannels {
        #[serde(default = "default_server_id")]
        server_id: String,
    },
    GetMembers {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    ListServers,
    CreateServer {
        name: String,
        icon_url: Option<String>,
    },
    JoinServer {
        server_id: String,
    },
    LeaveServer {
        server_id: String,
    },
    CreateChannel {
        server_id: String,
        name: String,
        category_id: Option<String>,
        is_private: Option<bool>,
        channel_type: Option<String>,
    },
    DeleteChannel {
        server_id: String,
        channel: String,
    },
    DeleteServer {
        server_id: String,
    },
    UpdateServer {
        server_id: String,
        name: Option<String>,
        icon_url: Option<String>,
    },
    UpdateMemberRole {
        server_id: String,
        user_id: String,
        role: String,
    },
    EditMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
        content: String,
        #[serde(default)]
        content_format: crate::engine::messaging::ContentFormat,
        #[serde(default)]
        mentions: Vec<crate::engine::messaging::MessageMention>,
    },
    DeleteMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
    },
    AddReaction {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
        emoji: String,
    },
    RemoveReaction {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
        emoji: String,
    },
    Typing {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    MarkRead {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        message_id: String,
    },
    GetUnreadCounts {
        #[serde(default = "default_server_id")]
        server_id: String,
    },
    // ── Roles ──
    ListRoles {
        server_id: String,
    },
    CreateRole {
        server_id: String,
        name: String,
        color: Option<String>,
        permissions: Option<i64>,
    },
    UpdateRole {
        server_id: String,
        role_id: String,
        name: String,
        color: Option<String>,
        permissions: i64,
    },
    DeleteRole {
        server_id: String,
        role_id: String,
    },
    AssignRole {
        server_id: String,
        user_id: String,
        role_id: String,
    },
    RemoveRole {
        server_id: String,
        user_id: String,
        role_id: String,
    },
    ListChannelPermissionOverrides {
        server_id: String,
        channel_id: String,
    },
    SetChannelPermissionOverride {
        server_id: String,
        channel_id: String,
        target_type: String,
        target_id: String,
        allow_bits: i64,
        deny_bits: i64,
    },
    DeleteChannelPermissionOverride {
        server_id: String,
        channel_id: String,
        target_type: String,
        target_id: String,
    },
    // ── Categories ──
    ListCategories {
        server_id: String,
    },
    CreateCategory {
        server_id: String,
        name: String,
    },
    UpdateCategory {
        server_id: String,
        category_id: String,
        name: String,
    },
    DeleteCategory {
        server_id: String,
        category_id: String,
    },
    // ── Channel organization ──
    ReorderChannels {
        server_id: String,
        channels: Vec<crate::engine::events::ChannelPositionInfo>,
    },
    // ── Phase 4: Presence ──
    SetPresence {
        status: String,
        custom_status: Option<String>,
        status_emoji: Option<String>,
    },
    GetPresences {
        server_id: String,
    },
    // ── Phase 4: Server Nicknames ──
    SetServerNickname {
        server_id: String,
        nickname: Option<String>,
    },
    // ── Phase 4: Search ──
    SearchMessages {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        server_id: String,
        query: String,
        channel: Option<String>,
        limit: Option<i64>,
        offset: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<String>,
    },
    // ── Phase 4: Notifications ──
    UpdateNotificationSettings {
        server_id: String,
        channel_id: Option<String>,
        level: String,
        suppress_everyone: Option<bool>,
        suppress_roles: Option<bool>,
        muted: Option<bool>,
        mute_until: Option<String>,
    },
    GetNotificationSettings {
        server_id: String,
    },
    // ── Phase 4: Profiles ──
    GetUserProfile {
        user_id: String,
    },
    // ── Phase 5: Pinning ──
    PinMessage {
        server_id: String,
        channel: String,
        message_id: String,
    },
    UnpinMessage {
        server_id: String,
        channel: String,
        message_id: String,
    },
    GetPinnedMessages {
        server_id: String,
        channel: String,
    },
    // ── Phase 5: Threads ──
    CreateThread {
        server_id: String,
        parent_channel: String,
        name: String,
        message_id: String,
        #[serde(default)]
        is_private: bool,
    },
    ArchiveThread {
        server_id: String,
        thread_id: String,
    },
    UnarchiveThread {
        server_id: String,
        thread_id: String,
    },
    ListThreads {
        server_id: String,
        channel: String,
    },
    CreateForumTag {
        server_id: String,
        channel: String,
        name: String,
        emoji: Option<String>,
        #[serde(default)]
        moderated: bool,
    },
    UpdateForumTag {
        server_id: String,
        channel: String,
        tag_id: String,
        name: String,
        emoji: Option<String>,
        moderated: bool,
        position: i32,
    },
    DeleteForumTag {
        server_id: String,
        channel: String,
        tag_id: String,
    },
    ListForumTags {
        server_id: String,
        channel: String,
    },
    SetThreadTags {
        server_id: String,
        thread_id: String,
        tag_ids: Vec<String>,
    },
    GetThreadTags {
        server_id: String,
        thread_id: String,
    },
    // ── Phase 5: Bookmarks ──
    AddBookmark {
        message_id: String,
        note: Option<String>,
    },
    RemoveBookmark {
        message_id: String,
    },
    ListBookmarks,
    // ── Phase 6: Moderation ──
    KickMember {
        server_id: String,
        user_id: String,
        reason: Option<String>,
    },
    BanMember {
        server_id: String,
        user_id: String,
        reason: Option<String>,
        #[serde(default)]
        delete_message_days: i32,
    },
    UnbanMember {
        server_id: String,
        user_id: String,
    },
    ListBans {
        server_id: String,
    },
    TimeoutMember {
        server_id: String,
        user_id: String,
        timeout_until: Option<String>,
        reason: Option<String>,
    },
    SetSlowMode {
        server_id: String,
        channel: String,
        seconds: i32,
    },
    SetNsfw {
        server_id: String,
        channel: String,
        is_nsfw: bool,
    },
    BulkDeleteMessages {
        server_id: String,
        channel: String,
        message_ids: Vec<String>,
    },
    GetAuditLog {
        server_id: String,
        action_type: Option<String>,
        limit: Option<i64>,
        before: Option<String>,
    },
    // ── Phase 6: AutoMod ──
    CreateAutomodRule {
        server_id: String,
        name: String,
        rule_type: String,
        config: String,
        action_type: String,
        timeout_duration_seconds: Option<i32>,
    },
    UpdateAutomodRule {
        server_id: String,
        rule_id: String,
        name: String,
        enabled: bool,
        config: String,
        action_type: String,
        timeout_duration_seconds: Option<i32>,
    },
    DeleteAutomodRule {
        server_id: String,
        rule_id: String,
    },
    ListAutomodRules {
        server_id: String,
    },
    // ── Phase 7: Community & Discovery ──
    CreateInvite {
        server_id: String,
        max_uses: Option<i32>,
        expires_at: Option<String>,
        channel_id: Option<String>,
    },
    ListInvites {
        server_id: String,
    },
    DeleteInvite {
        server_id: String,
        invite_id: String,
    },
    UseInvite {
        code: String,
    },
    CreateEvent {
        server_id: String,
        name: String,
        description: Option<String>,
        channel_id: Option<String>,
        start_time: String,
        end_time: Option<String>,
        image_url: Option<String>,
    },
    ListEvents {
        server_id: String,
    },
    UpdateEventStatus {
        server_id: String,
        event_id: String,
        status: String,
    },
    DeleteEvent {
        server_id: String,
        event_id: String,
    },
    SetRsvp {
        server_id: String,
        event_id: String,
        status: String,
    },
    RemoveRsvp {
        server_id: String,
        event_id: String,
    },
    ListRsvps {
        event_id: String,
    },
    UpdateCommunitySettings {
        server_id: String,
        description: Option<String>,
        is_discoverable: bool,
        welcome_message: Option<String>,
        rules_text: Option<String>,
        category: Option<String>,
    },
    GetCommunitySettings {
        server_id: String,
    },
    DiscoverServers {
        category: Option<String>,
    },
    AcceptRules {
        server_id: String,
    },
    SetAnnouncementChannel {
        server_id: String,
        channel: String,
        is_announcement: bool,
    },
    FollowChannel {
        source_channel_id: String,
        target_channel_id: String,
    },
    UnfollowChannel {
        follow_id: String,
    },
    ListChannelFollows {
        channel_id: String,
    },
    PublishAnnouncement {
        message_id: String,
    },
    CreateTemplate {
        server_id: String,
        name: String,
        description: Option<String>,
    },
    ListTemplates {
        server_id: String,
    },
    DeleteTemplate {
        server_id: String,
        template_id: String,
    },
    InstantiateTemplate {
        template_id: String,
        server_name: String,
    },
    // ── Phase 8: Integrations & Bots ──
    CreateWebhook {
        server_id: String,
        channel_id: String,
        name: String,
        webhook_type: String,
        url: Option<String>,
    },
    ListWebhooks {
        server_id: String,
    },
    UpdateWebhook {
        webhook_id: String,
        name: String,
        avatar_url: Option<String>,
        channel_id: String,
    },
    DeleteWebhook {
        webhook_id: String,
    },
    CreateBot {
        username: String,
        avatar_url: Option<String>,
    },
    ListOwnedBots,
    CreateBotToken {
        bot_user_id: String,
        name: String,
        scopes: Option<String>,
    },
    ListBotTokens {
        bot_user_id: String,
    },
    DeleteBotToken {
        token_id: String,
    },
    AddBotToServer {
        server_id: String,
        bot_user_id: String,
    },
    RemoveBotFromServer {
        server_id: String,
        bot_user_id: String,
    },
    RegisterSlashCommand {
        server_id: String,
        name: String,
        description: String,
        options_json: Option<String>,
    },
    ListSlashCommands {
        server_id: String,
    },
    DeleteSlashCommand {
        command_id: String,
    },
    InvokeSlashCommand {
        request_id: String,
        server_id: String,
        channel: String,
        command_name: String,
        args_json: Option<String>,
    },
    InvokeMessageComponent {
        request_id: String,
        message_id: String,
        custom_id: String,
        #[serde(default)]
        values: Vec<String>,
    },
    RespondToInteraction {
        interaction_id: String,
        content: Option<String>,
        embeds_json: Option<String>,
        components_json: Option<String>,
        ephemeral: Option<bool>,
    },
    CreateOAuth2App {
        name: String,
        description: Option<String>,
        redirect_uris: Vec<String>,
        #[serde(default = "default_oauth_client_type")]
        client_type: String,
    },
    ListOAuth2Apps,
    DeleteOAuth2App {
        app_id: String,
    },

    // ── Premium-for-Free features ──
    SetServerAvatar {
        server_id: String,
        avatar_url: Option<String>,
    },
    SetVanityCode {
        server_id: String,
        vanity_code: Option<String>,
    },
    GetServerLimits,
}

pub(super) fn default_server_id() -> String {
    DEFAULT_SERVER_ID.to_string()
}
