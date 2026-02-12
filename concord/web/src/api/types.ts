// ── Server types ────────────────────────────────────────

export interface UserProfile {
  id: string;
  username: string;
  email: string | null;
  avatar_url: string | null;
  is_system_admin?: boolean;
}

export interface AuthStatus {
  authenticated: boolean;
  providers: string[];
}

export interface ServerInfo {
  id: string;
  name: string;
  icon_url?: string | null;
  member_count: number;
  role?: string | null;
}

export interface ChannelInfo {
  id: string;
  server_id: string;
  name: string;
  topic: string;
  member_count: number;
  category_id?: string | null;
  position: number;
  is_private: boolean;
  channel_type: string;
  thread_parent_message_id?: string | null;
  archived: boolean;
  slowmode_seconds: number;
  is_nsfw: boolean;
}

export interface MemberInfo {
  nickname: string;
  avatar_url?: string | null;
  status?: string | null;
  custom_status?: string | null;
  status_emoji?: string | null;
  user_id?: string | null;
}

export interface ReplyInfo {
  id: string;
  from: string;
  content_preview: string;
}

export interface ReactionGroup {
  emoji: string;
  count: number;
  user_ids: string[];
}

export interface AttachmentInfo {
  id: string;
  filename: string;
  content_type: string;
  file_size: number;
  url: string;
}

export interface EmbedInfo {
  url: string;
  title?: string | null;
  description?: string | null;
  image_url?: string | null;
  site_name?: string | null;
}

export interface HistoryMessage {
  id: string;
  from: string;
  content: string;
  timestamp: string;
  edited_at?: string | null;
  reply_to?: ReplyInfo | null;
  reactions?: ReactionGroup[] | null;
  attachments?: AttachmentInfo[] | null;
  embeds?: EmbedInfo[] | null;
}

export interface UnreadCount {
  channel_name: string;
  count: number;
}

export interface RoleInfo {
  id: string;
  server_id: string;
  name: string;
  color?: string | null;
  icon_url?: string | null;
  position: number;
  permissions: number;
  is_default: boolean;
}

export interface CategoryInfo {
  id: string;
  server_id: string;
  name: string;
  position: number;
}

export interface ChannelPositionInfo {
  id: string;
  category_id?: string | null;
  position: number;
}

export interface PresenceInfo {
  user_id: string;
  nickname: string;
  avatar_url?: string | null;
  status: string; // 'online' | 'idle' | 'dnd' | 'offline'
  custom_status?: string | null;
  status_emoji?: string | null;
}

export interface UserProfileInfo {
  user_id: string;
  username: string;
  avatar_url?: string | null;
  bio?: string | null;
  pronouns?: string | null;
  banner_url?: string | null;
  created_at: string;
}

export interface NotificationSettingInfo {
  id: string;
  server_id?: string | null;
  channel_id?: string | null;
  level: string; // 'all' | 'mentions' | 'none' | 'default'
  suppress_everyone: boolean;
  suppress_roles: boolean;
  muted: boolean;
  mute_until?: string | null;
}

export interface SearchResultMessage {
  id: string;
  from: string;
  content: string;
  timestamp: string;
  channel_id: string;
  channel_name: string;
  edited_at?: string | null;
}

export interface PinnedMessageInfo {
  id: string;
  message_id: string;
  channel_id: string;
  pinned_by: string;
  pinned_at: string;
  from: string;
  content: string;
  timestamp: string;
}

export interface ThreadInfo {
  id: string;
  name: string;
  channel_type: string; // 'public_thread' | 'private_thread'
  parent_message_id?: string | null;
  archived: boolean;
  auto_archive_minutes: number;
  message_count: number;
  created_at: string;
}

export interface ForumTagInfo {
  id: string;
  name: string;
  emoji?: string | null;
  moderated: boolean;
  position: number;
}

export interface BookmarkInfo {
  id: string;
  message_id: string;
  channel_id: string;
  from: string;
  content: string;
  timestamp: string;
  note?: string | null;
  created_at: string;
}

export interface AuditLogEntry {
  id: string;
  actor_id: string;
  action_type: string;
  target_type?: string | null;
  target_id?: string | null;
  reason?: string | null;
  changes?: string | null;
  created_at: string;
}

export interface BanInfo {
  id: string;
  user_id: string;
  banned_by: string;
  reason?: string | null;
  created_at: string;
}

export interface AutomodRuleInfo {
  id: string;
  name: string;
  enabled: boolean;
  rule_type: string; // 'keyword' | 'mention_spam' | 'link_filter'
  config: string; // JSON string
  action_type: string; // 'delete' | 'timeout' | 'flag'
  timeout_duration_seconds?: number | null;
}

export interface InviteInfo {
  id: string;
  code: string;
  server_id: string;
  created_by: string;
  max_uses?: number | null;
  use_count: number;
  expires_at?: string | null;
  channel_id?: string | null;
  created_at: string;
}

export interface EventInfo {
  id: string;
  server_id: string;
  name: string;
  description?: string | null;
  channel_id?: string | null;
  start_time: string;
  end_time?: string | null;
  image_url?: string | null;
  created_by: string;
  status: string; // 'scheduled' | 'active' | 'completed' | 'cancelled'
  interested_count: number;
  created_at: string;
}

export interface RsvpInfo {
  user_id: string;
  status: string; // 'interested' | 'going'
}

export interface ChannelFollowInfo {
  id: string;
  source_channel_id: string;
  target_channel_id: string;
  created_by: string;
}

export interface TemplateInfo {
  id: string;
  name: string;
  description?: string | null;
  server_id: string;
  created_by: string;
  use_count: number;
  created_at: string;
}

export interface ServerCommunityInfo {
  server_id: string;
  description?: string | null;
  is_discoverable: boolean;
  welcome_message?: string | null;
  rules_text?: string | null;
  category?: string | null;
}

// ── Phase 8: Integrations & Bots ──────────────────────

export interface WebhookInfo {
  id: string;
  server_id: string;
  channel_id: string;
  name: string;
  avatar_url?: string | null;
  webhook_type: string; // 'incoming' | 'outgoing'
  token: string;
  url?: string | null;
  created_by: string;
  created_at: string;
}

export interface SlashCommandInfo {
  id: string;
  bot_user_id: string;
  server_id?: string | null;
  name: string;
  description: string;
  options: SlashCommandOption[];
  created_at: string;
}

export interface SlashCommandOption {
  name: string;
  description: string;
  option_type: string; // 'string' | 'integer' | 'boolean' | 'user' | 'channel' | 'role'
  required: boolean;
  choices?: SlashCommandChoice[];
}

export interface SlashCommandChoice {
  name: string;
  value: string;
}

export interface InteractionInfo {
  id: string;
  interaction_type: string;
  command_id?: string | null;
  command_name?: string | null;
  user_id: string;
  server_id: string;
  channel_id: string;
  data_json: string;
}

export interface BotTokenInfo {
  id: string;
  name: string;
  scopes: string;
  created_at: string;
  last_used?: string | null;
}

export interface OAuth2AppInfo {
  id: string;
  name: string;
  description: string;
  icon_url?: string | null;
  owner_id: string;
  redirect_uris: string;
  scopes: string;
  is_public: boolean;
  created_at: string;
}

export interface RichEmbedInfo {
  title?: string | null;
  description?: string | null;
  url?: string | null;
  color?: number | null;
  fields?: EmbedField[];
  footer?: { text: string; icon_url?: string | null } | null;
  author?: { name: string; url?: string | null; icon_url?: string | null } | null;
  thumbnail_url?: string | null;
  image_url?: string | null;
  timestamp?: string | null;
}

export interface EmbedField {
  name: string;
  value: string;
  inline: boolean;
}

export type MessageComponent =
  | { type: 'action_row'; components: MessageComponent[] }
  | { type: 'button'; custom_id: string; label: string; style: number; disabled?: boolean; url?: string | null }
  | { type: 'select_menu'; custom_id: string; placeholder?: string | null; min_values?: number; max_values?: number; options: SelectOption[] };

export interface SelectOption {
  label: string;
  value: string;
  description?: string | null;
  default?: boolean;
}

// ── Phase 9: AT Protocol Deep Integration ─────────────

export interface BlueskyIdentityInfo {
  did: string;
  bsky_handle: string | null;
  display_name: string | null;
  description: string | null;
  avatar_url: string | null;
  banner_url: string | null;
  followers_count: number | null;
  follows_count: number | null;
  last_profile_sync: string | null;
}

export interface BlueskyShareResult {
  post_uri: string;
}

// ── Permission bitfield constants ──────────────────────
export const Permissions = {
  VIEW_CHANNELS:        1 << 0,
  MANAGE_CHANNELS:      1 << 1,
  MANAGE_ROLES:         1 << 2,
  MANAGE_SERVER:        1 << 3,
  CREATE_INVITES:       1 << 4,
  KICK_MEMBERS:         1 << 5,
  BAN_MEMBERS:          1 << 6,
  ADMINISTRATOR:        1 << 7,
  SEND_MESSAGES:        1 << 10,
  EMBED_LINKS:          1 << 11,
  ATTACH_FILES:         1 << 12,
  ADD_REACTIONS:        1 << 13,
  MENTION_EVERYONE:     1 << 14,
  MANAGE_MESSAGES:      1 << 15,
  READ_MESSAGE_HISTORY: 1 << 16,
} as const;

export function hasPermission(perms: number, flag: number): boolean {
  if (perms & Permissions.ADMINISTRATOR) return true;
  return (perms & flag) === flag;
}

export interface PublicUserProfile {
  username: string;
  avatar_url: string | null;
  provider: string | null;
  provider_id: string | null;
}

export interface HistoryResponse {
  channel: string;
  messages: HistoryMessage[];
  has_more: boolean;
}

export interface IrcToken {
  id: string;
  label: string | null;
  last_used: string | null;
  created_at: string;
}

export interface CreateTokenResponse {
  id: string;
  token: string;
  label: string | null;
}

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
  | { type: 'role_list'; server_id: string; roles: RoleInfo[] }
  | { type: 'role_update'; server_id: string; role: RoleInfo }
  | { type: 'role_delete'; server_id: string; role_id: string }
  | { type: 'member_role_update'; server_id: string; user_id: string; role_ids: string[] }
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
  | { type: 'webhook_list'; server_id: string; webhooks: WebhookInfo[] }
  | { type: 'webhook_update'; server_id: string; webhook: WebhookInfo }
  | { type: 'webhook_delete'; server_id: string; webhook_id: string }
  | { type: 'slash_command_list'; server_id: string; commands: SlashCommandInfo[] }
  | { type: 'slash_command_update'; server_id: string; command: SlashCommandInfo }
  | { type: 'slash_command_delete'; server_id: string; command_id: string }
  | { type: 'interaction_create'; interaction: InteractionInfo }
  | { type: 'interaction_response'; interaction_id: string; response: { content?: string; embeds?: RichEmbedInfo[]; components?: MessageComponent[] } }
  | { type: 'bot_token_list'; tokens: BotTokenInfo[] }
  | { type: 'oauth2_app_list'; apps: OAuth2AppInfo[] }
  | { type: 'oauth2_app_update'; app: OAuth2AppInfo }
  | { type: 'error'; code: string; message: string };

// Client → Server commands
export type ClientCommand =
  | { type: 'send_message'; server_id: string; channel: string; content: string; reply_to?: string; attachment_ids?: string[]; nonce?: string }
  | { type: 'edit_message'; message_id: string; content: string }
  | { type: 'delete_message'; message_id: string }
  | { type: 'add_reaction'; message_id: string; emoji: string }
  | { type: 'remove_reaction'; message_id: string; emoji: string }
  | { type: 'typing'; server_id: string; channel: string }
  | { type: 'join_channel'; server_id: string; channel: string }
  | { type: 'part_channel'; server_id: string; channel: string; reason?: string }
  | { type: 'set_topic'; server_id: string; channel: string; topic: string }
  | { type: 'fetch_history'; server_id: string; channel: string; before?: string; limit?: number }
  | { type: 'list_channels'; server_id: string }
  | { type: 'get_members'; server_id: string; channel: string }
  | { type: 'list_servers' }
  | { type: 'create_server'; name: string; icon_url?: string }
  | { type: 'join_server'; server_id: string }
  | { type: 'leave_server'; server_id: string }
  | { type: 'create_channel'; server_id: string; name: string; category_id?: string; is_private?: boolean }
  | { type: 'delete_channel'; server_id: string; channel: string }
  | { type: 'delete_server'; server_id: string }
  | { type: 'update_member_role'; server_id: string; user_id: string; role: string }
  | { type: 'mark_read'; server_id: string; channel: string; message_id: string }
  | { type: 'get_unread_counts'; server_id: string }
  | { type: 'list_roles'; server_id: string }
  | { type: 'create_role'; server_id: string; name: string; color?: string; permissions?: number; position?: number }
  | { type: 'update_role'; server_id: string; role_id: string; name?: string; color?: string; permissions?: number; position?: number }
  | { type: 'delete_role'; server_id: string; role_id: string }
  | { type: 'assign_role'; server_id: string; user_id: string; role_id: string }
  | { type: 'remove_role'; server_id: string; user_id: string; role_id: string }
  | { type: 'list_categories'; server_id: string }
  | { type: 'create_category'; server_id: string; name: string }
  | { type: 'update_category'; server_id: string; category_id: string; name?: string; position?: number }
  | { type: 'delete_category'; server_id: string; category_id: string }
  | { type: 'reorder_channels'; server_id: string; channels: ChannelPositionInfo[] }
  | { type: 'set_presence'; status: string; custom_status?: string; status_emoji?: string }
  | { type: 'get_presences'; server_id: string }
  | { type: 'set_server_nickname'; server_id: string; nickname?: string }
  | { type: 'search_messages'; server_id: string; query: string; channel?: string; limit?: number; offset?: number }
  | { type: 'update_notification_settings'; server_id: string; channel_id?: string; level: string; suppress_everyone?: boolean; suppress_roles?: boolean; muted?: boolean; mute_until?: string }
  | { type: 'get_notification_settings'; server_id: string }
  | { type: 'get_user_profile'; user_id: string }
  | { type: 'pin_message'; server_id: string; channel: string; message_id: string }
  | { type: 'unpin_message'; server_id: string; channel: string; message_id: string }
  | { type: 'get_pinned_messages'; server_id: string; channel: string }
  | { type: 'create_thread'; server_id: string; parent_channel: string; name: string; message_id: string; is_private?: boolean }
  | { type: 'archive_thread'; server_id: string; thread_id: string }
  | { type: 'list_threads'; server_id: string; channel: string }
  | { type: 'add_bookmark'; message_id: string; note?: string }
  | { type: 'remove_bookmark'; message_id: string }
  | { type: 'list_bookmarks' }
  | { type: 'kick_member'; server_id: string; user_id: string; reason?: string }
  | { type: 'ban_member'; server_id: string; user_id: string; reason?: string; delete_message_days?: number }
  | { type: 'unban_member'; server_id: string; user_id: string }
  | { type: 'list_bans'; server_id: string }
  | { type: 'timeout_member'; server_id: string; user_id: string; timeout_until?: string; reason?: string }
  | { type: 'set_slow_mode'; server_id: string; channel: string; seconds: number }
  | { type: 'set_nsfw'; server_id: string; channel: string; is_nsfw: boolean }
  | { type: 'bulk_delete_messages'; server_id: string; channel: string; message_ids: string[] }
  | { type: 'get_audit_log'; server_id: string; action_type?: string; limit?: number; before?: string }
  | { type: 'create_automod_rule'; server_id: string; name: string; rule_type: string; config: string; action_type: string; timeout_duration_seconds?: number }
  | { type: 'update_automod_rule'; server_id: string; rule_id: string; name: string; enabled: boolean; config: string; action_type: string; timeout_duration_seconds?: number }
  | { type: 'delete_automod_rule'; server_id: string; rule_id: string }
  | { type: 'list_automod_rules'; server_id: string }
  | { type: 'create_invite'; server_id: string; max_uses?: number; expires_at?: string; channel_id?: string }
  | { type: 'list_invites'; server_id: string }
  | { type: 'delete_invite'; server_id: string; invite_id: string }
  | { type: 'use_invite'; code: string }
  | { type: 'create_event'; server_id: string; name: string; description?: string; channel_id?: string; start_time: string; end_time?: string; image_url?: string }
  | { type: 'list_events'; server_id: string }
  | { type: 'update_event_status'; server_id: string; event_id: string; status: string }
  | { type: 'delete_event'; server_id: string; event_id: string }
  | { type: 'set_rsvp'; server_id: string; event_id: string; status: string }
  | { type: 'remove_rsvp'; server_id: string; event_id: string }
  | { type: 'list_rsvps'; event_id: string }
  | { type: 'update_community_settings'; server_id: string; description?: string; is_discoverable: boolean; welcome_message?: string; rules_text?: string; category?: string }
  | { type: 'get_community_settings'; server_id: string }
  | { type: 'discover_servers'; category?: string }
  | { type: 'accept_rules'; server_id: string }
  | { type: 'set_announcement_channel'; server_id: string; channel: string; is_announcement: boolean }
  | { type: 'follow_channel'; source_channel_id: string; target_channel_id: string }
  | { type: 'unfollow_channel'; follow_id: string }
  | { type: 'list_channel_follows'; channel_id: string }
  | { type: 'create_template'; server_id: string; name: string; description?: string }
  | { type: 'list_templates'; server_id: string }
  | { type: 'delete_template'; server_id: string; template_id: string }
  // Phase 8: Integrations & Bots
  | { type: 'create_webhook'; server_id: string; channel_id: string; name: string; webhook_type: string; url?: string }
  | { type: 'list_webhooks'; server_id: string }
  | { type: 'update_webhook'; webhook_id: string; name: string; avatar_url?: string }
  | { type: 'delete_webhook'; webhook_id: string }
  | { type: 'create_bot'; username: string }
  | { type: 'create_bot_token'; bot_user_id: string; name?: string; scopes?: string }
  | { type: 'list_bot_tokens'; bot_user_id: string }
  | { type: 'delete_bot_token'; token_id: string }
  | { type: 'add_bot_to_server'; bot_user_id: string; server_id: string }
  | { type: 'remove_bot_from_server'; bot_user_id: string; server_id: string }
  | { type: 'register_slash_command'; server_id: string; name: string; description: string; options_json?: string }
  | { type: 'list_slash_commands'; server_id: string }
  | { type: 'delete_slash_command'; command_id: string }
  | { type: 'invoke_slash_command'; server_id: string; channel_id: string; command_name: string; args_json?: string }
  | { type: 'respond_to_interaction'; interaction_id: string; content?: string; embeds_json?: string; components_json?: string }
  | { type: 'create_oauth2_app'; name: string; description: string; redirect_uris: string; scopes?: string }
  | { type: 'list_oauth2_apps' }
  | { type: 'delete_oauth2_app'; app_id: string }
  | { type: 'update_server'; server_id: string; name?: string; icon_url?: string };

// ── Helpers ─────────────────────────────────────────────

/** Composite key for channel-scoped data: "server_id:channel_name" */
export function channelKey(serverId: string, channel: string): string {
  return `${serverId}:${channel}`;
}
