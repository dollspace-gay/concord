import type { ChannelPermissionOverrideInfo, DirectConversationInfo } from '../../api/generated/contract';
import type { BlueskyIdentityInfo, BookmarkInfo, BotTokenInfo, CategoryInfo, ChannelInfo, EventInfo, ForumTagInfo, HistoryMessage, InviteInfo, MemberInfo, NotificationSettingInfo, OAuth2AppInfo, PinnedMessageInfo, PresenceInfo, RoleInfo, ServerCommunityInfo, ServerInfo, SlashCommandInfo, TemplateInfo, ThreadInfo, UserProfileInfo, WebhookInfo } from '../../api/types';

/** Maximum messages retained per channel to prevent unbounded memory growth. */
export const MAX_MESSAGES_PER_CHANNEL = 1000;

export const LIFECYCLE_RESULT_TIMEOUT_MS = 15_000;

export const UNCERTAIN_LIFECYCLE_MESSAGE = 'Connection closed after the action was sent. Its result is unknown; refresh current state before retrying.';

// Stable empty references to prevent zustand selector re-render loops.
// Inline [] / {} in selectors create new references on every evaluation,
// failing Object.is comparison and causing infinite re-renders with React 19.
export const EMPTY_SERVERS: ServerInfo[] = [];

export const EMPTY_CHANNELS_MAP: Record<string, ChannelInfo[]> = {};

export const EMPTY_MESSAGES_MAP: Record<string, HistoryMessage[]> = {};

export const EMPTY_MEMBERS_MAP: Record<string, MemberInfo[]> = {};

export const EMPTY_HAS_MORE: Record<string, boolean> = {};

export const EMPTY_AVATARS: Record<string, string> = {};

export const EMPTY_TYPING: Record<string, string[]> = {};

export const EMPTY_UNREAD: Record<string, number> = {};

export const EMPTY_READ_SEQUENCES: Record<string, string> = {};

export const EMPTY_EMOJI: Record<string, Record<string, { id: string; image_url: string }>> = {};

export const EMPTY_ROLES: Record<string, RoleInfo[]> = {};

export const EMPTY_CHANNEL_OVERRIDES: Record<string, ChannelPermissionOverrideInfo[]> = {};

export const EMPTY_CATEGORIES: Record<string, CategoryInfo[]> = {};

export const EMPTY_PRESENCES: Record<string, Record<string, PresenceInfo>> = {};

export const EMPTY_PROFILES: Record<string, UserProfileInfo> = {};

export const EMPTY_PINS: Record<string, PinnedMessageInfo[]> = {};

export const EMPTY_THREADS: Record<string, ThreadInfo[]> = {};

export const EMPTY_FORUM_TAGS: Record<string, ForumTagInfo[]> = {};

export const EMPTY_BOOKMARKS: BookmarkInfo[] = [];

export const EMPTY_NOTIFICATION_SETTINGS: Record<string, NotificationSettingInfo[]> = {};

export const EMPTY_DIRECT_CONVERSATIONS: DirectConversationInfo[] = [];

export const EMPTY_INVITES: Record<string, InviteInfo[]> = {};

export const EMPTY_EVENTS: Record<string, EventInfo[]> = {};

export const EMPTY_COMMUNITY: Record<string, ServerCommunityInfo> = {};

export const EMPTY_DISCOVER: ServerCommunityInfo[] = [];

export const EMPTY_TEMPLATES: Record<string, TemplateInfo[]> = {};

export const EMPTY_WEBHOOKS: Record<string, WebhookInfo[]> = {};

export const EMPTY_SLASH_COMMANDS: Record<string, SlashCommandInfo[]> = {};

export const EMPTY_BOT_TOKENS: BotTokenInfo[] = [];

export const EMPTY_OAUTH2_APPS: OAuth2AppInfo[] = [];

export const EMPTY_BLUESKY_IDENTITIES: Record<string, BlueskyIdentityInfo> = {};

export const EMPTY_MEMBER_ROLES: Record<string, Record<string, string[]>> = {};
