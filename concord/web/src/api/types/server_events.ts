import type { ChannelFollowInfo, EventInfo, InviteInfo, RsvpInfo, ServerCommunityInfo, TemplateInfo } from './community';
import type { BookmarkInfo, ForumTagInfo, PinnedMessageInfo, SearchResultMessage, ThreadInfo } from './conversations';
import type { BotAccountInfo, BotTokenInfo, InteractionInfo, OAuth2AppInfo, SlashCommandInfo, WebhookInfo } from './integrations';
import type { AttachmentInfo, EmbedInfo, HistoryMessage, ReplyInfo, UnreadCount } from './messages';
import type { AuditLogEntry, AutomodRuleInfo, BanInfo } from './moderation';
import type { ChannelInfo, MemberInfo, ServerInfo } from './organization';
import type { CategoryInfo, ChannelPositionInfo, RoleInfo } from './permissions';
import type { NotificationSettingInfo, PresenceInfo, UserProfileInfo } from './profiles';
import type { MessageComponent, RichEmbedInfo } from './rich_messages';

// ── WebSocket message types ─────────────────────────────

// Server → Client events
export type ServerEvent =
  | { type: 'message'; id: string; server_id?: string; from: string; target: string; content: string; timestamp: string; avatar_url?: string; reply_to?: ReplyInfo | null; attachments?: AttachmentInfo[] | null }
  | { type: 'message_edit'; id: string; server_id: string; channel: string; content: string; edited_at: string }
  | { type: 'message_delete'; id: string; server_id: string; channel: string }
  | { type: 'message_ack'; id: string; server_id: string; channel: string; nonce?: string }
  | { type: 'message_embed'; message_id: string; server_id: string; channel: string; embeds: EmbedInfo[] }
  | { type: 'reaction_add'; message_id: string; server_id: string; channel: string; user_id: string; nickname: string; emoji: string }
  | { type: 'reaction_remove'; message_id: string; server_id: string; channel: string; user_id: string; nickname: string; emoji: string }
  | { type: 'typing_start'; server_id: string; channel: string; nickname: string }
  | { type: 'join'; nickname: string; server_id: string; channel: string; avatar_url?: string }
  | { type: 'part'; nickname: string; server_id: string; channel: string; reason?: string }
  | { type: 'quit'; nickname: string; reason?: string }
  | { type: 'topic_change'; server_id: string; channel: string; set_by: string; topic: string }
  | { type: 'nick_change'; old_nick: string; new_nick: string }
  | { type: 'names'; server_id: string; channel: string; members: MemberInfo[] }
  | { type: 'topic'; server_id: string; channel: string; topic: string }
  | { type: 'channel_list'; server_id: string; channels: ChannelInfo[] }
  | { type: 'history'; server_id: string; channel: string; messages: HistoryMessage[]; has_more: boolean }
  | { type: 'server_list'; servers: ServerInfo[] }
  | { type: 'unread_counts'; server_id: string; counts: UnreadCount[] }
  | { type: 'server_notice'; message: string }
  | { type: 'role_list'; server_id: string; version: number; roles: RoleInfo[]; member_roles?: Array<{ user_id: string; role_ids: string[] }> }
  | { type: 'role_update'; server_id: string; role: RoleInfo }
  | { type: 'role_delete'; server_id: string; role_id: string }
  | { type: 'member_role_update'; server_id: string; version: number; user_id: string; role_ids: string[] }
  | { type: 'category_list'; server_id: string; categories: CategoryInfo[] }
  | { type: 'category_update'; server_id: string; category: CategoryInfo }
  | { type: 'category_delete'; server_id: string; category_id: string }
  | { type: 'channel_reorder'; server_id: string; channels: ChannelPositionInfo[] }
  | { type: 'presence_update'; server_id: string; presence: PresenceInfo }
  | { type: 'presence_list'; server_id: string; presences: PresenceInfo[] }
  | { type: 'user_profile'; profile: UserProfileInfo }
  | { type: 'server_nickname_update'; server_id: string; user_id: string; nickname: string | null }
  | { type: 'notification_settings'; server_id: string; settings: NotificationSettingInfo[] }
  | { type: 'search_results'; server_id: string; query: string; results: SearchResultMessage[]; total_count: number; offset: number }
  | { type: 'message_pin'; server_id: string; channel: string; pin: PinnedMessageInfo }
  | { type: 'message_unpin'; server_id: string; channel: string; message_id: string }
  | { type: 'pinned_messages'; server_id: string; channel: string; pins: PinnedMessageInfo[] }
  | { type: 'thread_create'; server_id: string; parent_channel: string; thread: ThreadInfo }
  | { type: 'thread_update'; server_id: string; thread: ThreadInfo }
  | { type: 'thread_list'; server_id: string; channel: string; threads: ThreadInfo[] }
  | { type: 'forum_tag_list'; server_id: string; channel: string; tags: ForumTagInfo[] }
  | { type: 'forum_tag_update'; server_id: string; channel: string; tag: ForumTagInfo }
  | { type: 'forum_tag_delete'; server_id: string; channel: string; tag_id: string }
  | { type: 'bookmark_list'; bookmarks: BookmarkInfo[] }
  | { type: 'bookmark_add'; bookmark: BookmarkInfo }
  | { type: 'bookmark_remove'; message_id: string }
  | { type: 'member_kick'; server_id: string; user_id: string; kicked_by: string; reason?: string | null }
  | { type: 'member_ban'; server_id: string; user_id: string; banned_by: string; reason?: string | null }
  | { type: 'member_unban'; server_id: string; user_id: string }
  | { type: 'member_timeout'; server_id: string; user_id: string; timeout_until?: string | null }
  | { type: 'slow_mode_update'; server_id: string; channel: string; seconds: number }
  | { type: 'nsfw_update'; server_id: string; channel: string; is_nsfw: boolean }
  | { type: 'bulk_message_delete'; server_id: string; channel: string; message_ids: string[] }
  | { type: 'audit_log_entries'; server_id: string; entries: AuditLogEntry[] }
  | { type: 'ban_list'; server_id: string; bans: BanInfo[] }
  | { type: 'automod_rule_list'; server_id: string; rules: AutomodRuleInfo[] }
  | { type: 'automod_rule_update'; server_id: string; rule: AutomodRuleInfo }
  | { type: 'automod_rule_delete'; server_id: string; rule_id: string }
  | { type: 'invite_list'; server_id: string; invites: InviteInfo[] }
  | { type: 'invite_create'; server_id: string; invite: InviteInfo }
  | { type: 'invite_delete'; server_id: string; invite_id: string }
  | { type: 'event_list'; server_id: string; events: EventInfo[] }
  | { type: 'event_update'; server_id: string; event: EventInfo }
  | { type: 'event_delete'; server_id: string; event_id: string }
  | { type: 'event_rsvp_list'; event_id: string; rsvps: RsvpInfo[] }
  | { type: 'server_community'; community: ServerCommunityInfo }
  | { type: 'discover_servers'; servers: ServerCommunityInfo[] }
  | { type: 'channel_follow_list'; channel_id: string; follows: ChannelFollowInfo[] }
  | { type: 'channel_follow_create'; follow: ChannelFollowInfo }
  | { type: 'channel_follow_delete'; follow_id: string }
  | { type: 'template_list'; server_id: string; templates: TemplateInfo[] }
  | { type: 'template_update'; server_id: string; template: TemplateInfo }
  | { type: 'template_delete'; server_id: string; template_id: string }
  | { type: 'template_instantiated'; server_id: string; template_id: string }
  | { type: 'webhook_list'; server_id: string; webhooks: WebhookInfo[] }
  | { type: 'webhook_update'; server_id: string; webhook: WebhookInfo }
  | { type: 'webhook_delete'; server_id: string; webhook_id: string }
  | { type: 'slash_command_list'; server_id: string; commands: SlashCommandInfo[] }
  | { type: 'slash_command_update'; server_id: string; command: SlashCommandInfo }
  | { type: 'slash_command_delete'; server_id: string; command_id: string }
  | { type: 'interaction_create'; interaction: InteractionInfo }
  | { type: 'interaction_response'; interaction_id: string; response: { content?: string; embeds?: RichEmbedInfo[]; components?: MessageComponent[] } }
  | { type: 'bot_token_list'; bot_user_id?: string; tokens: BotTokenInfo[] }
  | { type: 'bot_account_list'; bots: BotAccountInfo[] }
  | { type: 'bot_credential_created'; bot_user_id: string; token: string; credential: BotTokenInfo }
  | { type: 'oauth2_app_list'; apps: OAuth2AppInfo[] }
  | { type: 'oauth2_app_update'; app: OAuth2AppInfo }
  | { type: 'server_avatar_update'; server_id: string; user_id: string; avatar_url: string | null }
  | { type: 'server_limits'; max_message_length: number; max_file_size_mb: number }
  | { type: 'error'; code: string; message: string };
