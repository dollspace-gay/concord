import { create } from 'zustand';
import type { AttachmentInfo, AuditLogEntry, AutomodRuleInfo, BanInfo, BlueskyIdentityInfo, BlueskyShareResult, BookmarkInfo, BotAccountInfo, BotTokenInfo, CategoryInfo, ChannelFollowInfo, ChannelInfo, ChannelPositionInfo, EventInfo, ForumTagInfo, HistoryMessage, InviteInfo, MemberInfo, NotificationSettingInfo, OAuth2AppInfo, PinnedMessageInfo, PresenceInfo, ReplyInfo, RoleInfo, RsvpInfo, SearchResultMessage, ServerCommunityInfo, ServerInfo, SlashCommandInfo, TemplateInfo, ThreadInfo, UserProfileInfo, WebhookInfo } from '../api/types';
import type { ChannelPermissionOverrideInfo, ChatEvent as ServerEvent, ClientMessage as ClientCommand, DirectConversationInfo, DurableEventProjection, DurableMessageProjection, SnapshotReactionGroup } from '../api/generated/contract';
import type { StickerInfo } from '../api/types';
import { listServerEmoji, createServerEmoji, deleteServerEmoji, syncBlueskyProfile, getBlueskyIdentity, shareToBluesky, getAtprotoSyncSetting, updateAtprotoSyncSetting, listServerStickers, createServerSticker, deleteServerSticker, listAllUserEmoji, getServerLimits } from '../api/client';
import { channelKey } from '../api/types';
import { WebSocketManager } from '../api/websocket';
import { useUiStore } from './uiStore';
import { useComposerStore, type FailedComposition } from './composerStore';
import { useConnectionStore, type ConnectionState } from './connectionStore';
import { LifecycleOutcomeUncertainError, usePendingStore } from './pendingStore';
import { useEntityStore } from './entityStore';

/** Maximum messages retained per channel to prevent unbounded memory growth. */
const MAX_MESSAGES_PER_CHANNEL = 1000;
const LIFECYCLE_RESULT_TIMEOUT_MS = 15_000;
const UNCERTAIN_LIFECYCLE_MESSAGE = 'Connection closed after the action was sent. Its result is unknown; refresh current state before retrying.';

// Stable empty references to prevent zustand selector re-render loops.
// Inline [] / {} in selectors create new references on every evaluation,
// failing Object.is comparison and causing infinite re-renders with React 19.
const EMPTY_SERVERS: ServerInfo[] = [];
const EMPTY_CHANNELS_MAP: Record<string, ChannelInfo[]> = {};
const EMPTY_MESSAGES_MAP: Record<string, HistoryMessage[]> = {};
const EMPTY_MEMBERS_MAP: Record<string, MemberInfo[]> = {};
const EMPTY_HAS_MORE: Record<string, boolean> = {};
const EMPTY_AVATARS: Record<string, string> = {};
const EMPTY_TYPING: Record<string, string[]> = {};
const EMPTY_UNREAD: Record<string, number> = {};
const EMPTY_READ_SEQUENCES: Record<string, string> = {};
const EMPTY_EMOJI: Record<string, Record<string, { id: string; image_url: string }>> = {};
const EMPTY_ROLES: Record<string, RoleInfo[]> = {};
const EMPTY_CHANNEL_OVERRIDES: Record<string, ChannelPermissionOverrideInfo[]> = {};
const EMPTY_CATEGORIES: Record<string, CategoryInfo[]> = {};
const EMPTY_PRESENCES: Record<string, Record<string, PresenceInfo>> = {};
const EMPTY_PROFILES: Record<string, UserProfileInfo> = {};
const EMPTY_PINS: Record<string, PinnedMessageInfo[]> = {};
const EMPTY_THREADS: Record<string, ThreadInfo[]> = {};
const EMPTY_FORUM_TAGS: Record<string, ForumTagInfo[]> = {};
const EMPTY_BOOKMARKS: BookmarkInfo[] = [];
const EMPTY_NOTIFICATION_SETTINGS: Record<string, NotificationSettingInfo[]> = {};
const EMPTY_DIRECT_CONVERSATIONS: DirectConversationInfo[] = [];
const EMPTY_INVITES: Record<string, InviteInfo[]> = {};
const EMPTY_EVENTS: Record<string, EventInfo[]> = {};
const EMPTY_COMMUNITY: Record<string, ServerCommunityInfo> = {};
const EMPTY_DISCOVER: ServerCommunityInfo[] = [];
const EMPTY_TEMPLATES: Record<string, TemplateInfo[]> = {};
const EMPTY_WEBHOOKS: Record<string, WebhookInfo[]> = {};
const EMPTY_SLASH_COMMANDS: Record<string, SlashCommandInfo[]> = {};
const EMPTY_BOT_TOKENS: BotTokenInfo[] = [];
const EMPTY_OAUTH2_APPS: OAuth2AppInfo[] = [];
const EMPTY_BLUESKY_IDENTITIES: Record<string, BlueskyIdentityInfo> = {};
const EMPTY_MEMBER_ROLES: Record<string, Record<string, string[]>> = {};

/** Tracks per-user typing indicator timeouts so they can be cleared on re-type. */
const typingTimeouts = new Map<string, ReturnType<typeof setTimeout>>();
const retriedCommands = new Set<string>();
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();
const pendingCommandOwners = new Map<string, number>();
const recoveredThisConnection = new Set<string>();
/** Prevent repeated channel-list projections from refetching identical server bootstrap data. */
const hydratedServerMetadata = new Set<string>();
const draftsByAccount = new Map<string, Record<string, string>>();

interface ChatState {
  connected: boolean;
  operationGeneration: string | null;
  syncCursor: string | null;
  syncWindowCursors: Record<string, string>;
  pendingCommands: Record<string, ClientCommand>;
  accountGeneration: number;
  /** Invalidates delayed protected HTTP reads after disconnect, resync, or account change. */
  protectedGeneration: number;
  durableMode: boolean;
  ownPresenceStatus: string | null;
  ownRequestedStatus: string | null;
  ownCustomStatus: string | null;
  ownStatusEmoji: string | null;
  entityVersions: Record<string, number>;
  readSequences: Record<string, string>;
  drafts: Record<string, string>;
  compositionFiles: Record<string, File[]>;
  failedCompositions: FailedComposition[];
  directConversations: DirectConversationInfo[];
  activeAccountId: string | null;
  nickname: string | null;
  servers: ServerInfo[];
  channels: Record<string, ChannelInfo[]>;   // server_id -> channels
  messages: Record<string, HistoryMessage[]>; // channelKey -> messages
  members: Record<string, MemberInfo[]>;      // channelKey -> members
  hasMore: Record<string, boolean>;           // channelKey -> has_more
  /** nickname -> avatar_url cache (populated from Names/Join/Message events) */
  avatars: Record<string, string>;
  /** channelKey -> list of nicknames currently typing */
  typingUsers: Record<string, string[]>;
  /** The message being replied to (if any) */
  replyingTo: ReplyInfo | null;
  /** channelKey -> unread message count */
  unreadCounts: Record<string, number>;
  /** server_id -> { emoji_name -> { id, image_url } } */
  customEmoji: Record<string, Record<string, { id: string; image_url: string }>>;
  /** server_id -> roles sorted by position desc */
  roles: Record<string, RoleInfo[]>;
  /** server_id -> categories sorted by position */
  categories: Record<string, CategoryInfo[]>;
  /** server_id -> user_id -> PresenceInfo */
  presences: Record<string, Record<string, PresenceInfo>>;
  /** Cached user profiles by user_id */
  userProfiles: Record<string, UserProfileInfo>;
  /** Search results */
  searchResults: SearchResultMessage[] | null;
  searchQuery: string;
  searchTotalCount: number;
  searchOffset: number;
  searchNextContinuation: string | null;
  searchContinuationTokens: Record<string, string>;
  activeSearchRequestId: string | null;
  searchRestarted: boolean;
  deletedMessageIds: Record<string, true>;
  /** channelKey -> pinned messages */
  pinnedMessages: Record<string, PinnedMessageInfo[]>;
  /** channelKey -> threads */
  threads: Record<string, ThreadInfo[]>;
  /** channelKey -> forum tags */
  forumTags: Record<string, ForumTagInfo[]>;
  /** Personal bookmarks */
  bookmarks: BookmarkInfo[];
  /** server_id -> global/server/channel notification preference rows */
  notificationSettings: Record<string, NotificationSettingInfo[]>;
  /** server_id -> user_id -> assigned custom role IDs */
  memberRoles: Record<string, Record<string, string[]>>;
  /** Highest accepted coherent role projection version per server. */
  roleProjectionVersions: Record<string, number>;
  /** channel_id -> role/member permission overrides */
  channelPermissionOverrides: Record<string, ChannelPermissionOverrideInfo[]>;
  /** server_id -> audit log entries */
  auditLog: Record<string, AuditLogEntry[]>;
  /** server_id -> ban list */
  bans: Record<string, BanInfo[]>;
  /** server_id -> automod rules */
  automodRules: Record<string, AutomodRuleInfo[]>;
  /** server_id -> invites */
  invites: Record<string, InviteInfo[]>;
  /** server_id -> scheduled events */
  serverEvents: Record<string, EventInfo[]>;
  /** event_id -> current RSVP rows */
  eventRsvps: Record<string, RsvpInfo[]>;
  /** server_id -> community settings */
  communitySettings: Record<string, ServerCommunityInfo>;
  /** Discoverable servers */
  discoverableServers: ServerCommunityInfo[];
  /** source channel_id -> outgoing announcement follows */
  channelFollows: Record<string, ChannelFollowInfo[]>;
  /** server_id -> templates */
  templates: Record<string, TemplateInfo[]>;
  /** server_id -> webhooks */
  webhooks: Record<string, WebhookInfo[]>;
  /** server_id -> slash commands */
  slashCommands: Record<string, SlashCommandInfo[]>;
  /** Bot tokens (for current user's bots) */
  botTokens: BotTokenInfo[];
  botAccounts: BotAccountInfo[];
  botCredential: { botUserId: string; token: string; credential: BotTokenInfo } | null;
  /** OAuth2 apps (for current user) */
  oauth2Apps: OAuth2AppInfo[];
  /** user_id -> Bluesky identity info (cached) */
  blueskyIdentities: Record<string, BlueskyIdentityInfo>;
  /** Whether current user has AT Protocol record sync enabled */
  atprotoSyncEnabled: boolean;
  /** server_id -> stickers */
  stickers: Record<string, StickerInfo[]>;
  /** All emoji from all user's servers (for cross-server emoji) */
  allUserEmoji: { server_id: string; name: string; image_url: string }[];
  /** server_id -> user_id -> avatar_url (per-server avatars) */
  serverAvatars: Record<string, Record<string, string>>;
  /** Configurable message length limit */
  maxMessageLength: number;
  /** Configurable max file size in MB */
  maxFileSizeMb: number;
  /** Error toast message (auto-clears) */
  errorToast: string | null;
  ws: WebSocketManager | null;
  sendTracked: (requestId: string, command: ClientCommand) => boolean;
  runLifecycleCommand: (command: ClientCommand, pendingKey: string) => Promise<void>;
  setDraft: (key: string, text: string) => void;
  setCompositionFiles: (key: string, files: File[]) => void;
  retryFailedComposition: (id: string) => boolean;
  dismissFailedComposition: (id: string) => void;

  connect: (nickname: string, accountId?: string) => void;
  disconnect: () => void;
  handleEvent: (event: ServerEvent) => void;
  sendMessage: (serverId: string, channel: string, content: string, attachments?: AttachmentInfo[]) => boolean;
  sendDirectMessage: (conversationId: string, recipient: string, content: string, attachments?: AttachmentInfo[]) => boolean;
  listDirectConversations: () => void;
  editMessage: (messageId: string, content: string) => void;
  deleteMessage: (messageId: string) => void;
  addReaction: (messageId: string, emoji: string) => void;
  removeReaction: (messageId: string, emoji: string) => void;
  sendTyping: (serverId: string, channel: string) => void;
  setReplyingTo: (reply: ReplyInfo | null) => void;
  markRead: (serverId: string, channel: string, messageId: string) => void;
  markDirectRead: (conversationId: string, messageId: string) => void;
  getUnreadCounts: (serverId: string) => void;
  joinChannel: (serverId: string, channel: string) => void;
  partChannel: (serverId: string, channel: string) => void;
  setTopic: (serverId: string, channel: string, topic: string) => void;
  fetchHistory: (serverId: string, channel: string, before?: string) => void;
  listChannels: (serverId: string) => void;
  getMembers: (serverId: string, channel: string) => void;
  listServers: () => void;
  createServer: (name: string, iconUrl?: string) => Promise<void>;
  joinServer: (serverId: string) => void;
  leaveServer: (serverId: string) => void;
  createChannel: (serverId: string, name: string, categoryId?: string, isPrivate?: boolean, channelType?: 'text' | 'forum') => void;
  deleteChannel: (serverId: string, channel: string) => void;
  deleteServer: (serverId: string) => Promise<void>;
  updateServer: (serverId: string, name?: string, iconUrl?: string) => Promise<void>;
  loadServerEmoji: (serverId: string) => Promise<void>;
  createEmoji: (serverId: string, name: string, imageUrl: string) => Promise<void>;
  deleteEmoji: (serverId: string, emojiId: string) => Promise<void>;
  listRoles: (serverId: string) => void;
  createRole: (serverId: string, name: string, color?: string, permissions?: number) => void;
  updateRole: (serverId: string, roleId: string, updates: { name?: string; color?: string; permissions?: number; position?: number }) => void;
  deleteRole: (serverId: string, roleId: string) => void;
  assignRole: (serverId: string, userId: string, roleId: string) => void;
  removeRole: (serverId: string, userId: string, roleId: string) => void;
  listChannelPermissionOverrides: (serverId: string, channelId: string) => void;
  setChannelPermissionOverride: (serverId: string, channelId: string, targetType: 'role' | 'user', targetId: string, allowBits: number, denyBits: number) => void;
  deleteChannelPermissionOverride: (serverId: string, channelId: string, targetType: 'role' | 'user', targetId: string) => void;
  listCategories: (serverId: string) => void;
  createCategory: (serverId: string, name: string) => void;
  updateCategory: (serverId: string, categoryId: string, updates: { name?: string; position?: number }) => void;
  deleteCategory: (serverId: string, categoryId: string) => void;
  reorderChannels: (serverId: string, channels: ChannelPositionInfo[]) => void;
  setPresence: (status: string, customStatus?: string, statusEmoji?: string) => void;
  getPresences: (serverId: string) => void;
  setServerNickname: (serverId: string, nickname?: string) => void;
  searchMessages: (serverId: string, query: string, channel?: string, limit?: number, offset?: number, continuation?: string) => void;
  clearSearch: () => void;
  updateNotificationSettings: (serverId: string, channelId: string | undefined, level: string, options?: { suppressEveryone?: boolean; suppressRoles?: boolean; muted?: boolean; muteUntil?: string }) => void;
  getNotificationSettings: (serverId: string) => void;
  getUserProfile: (userId: string) => void;
  pinMessage: (serverId: string, channel: string, messageId: string) => void;
  unpinMessage: (serverId: string, channel: string, messageId: string) => void;
  getPinnedMessages: (serverId: string, channel: string) => void;
  createThread: (serverId: string, parentChannel: string, name: string, messageId: string, isPrivate?: boolean) => void;
  archiveThread: (serverId: string, threadId: string) => void;
  unarchiveThread: (serverId: string, threadId: string) => void;
  listThreads: (serverId: string, channel: string) => void;
  createForumTag: (serverId: string, channel: string, name: string, emoji: string | undefined, moderated: boolean) => void;
  updateForumTag: (serverId: string, channel: string, tag: ForumTagInfo) => void;
  deleteForumTag: (serverId: string, channel: string, tagId: string) => void;
  listForumTags: (serverId: string, channel: string) => void;
  setThreadTags: (serverId: string, threadId: string, tagIds: string[]) => void;
  getThreadTags: (serverId: string, threadId: string) => void;
  addBookmark: (messageId: string, note?: string) => void;
  removeBookmark: (messageId: string) => void;
  listBookmarks: () => void;
  // ── Phase 6: Moderation ──
  kickMember: (serverId: string, userId: string, reason?: string) => Promise<void>;
  banMember: (serverId: string, userId: string, reason?: string, deleteMessageDays?: number) => Promise<void>;
  unbanMember: (serverId: string, userId: string) => Promise<void>;
  listBans: (serverId: string) => void;
  timeoutMember: (serverId: string, userId: string, timeoutUntil?: string, reason?: string) => Promise<void>;
  setSlowMode: (serverId: string, channel: string, seconds: number) => Promise<void>;
  setNsfw: (serverId: string, channel: string, isNsfw: boolean) => Promise<void>;
  bulkDeleteMessages: (serverId: string, channel: string, messageIds: string[]) => Promise<void>;
  getAuditLog: (serverId: string, actionType?: string, limit?: number, before?: string) => void;
  createAutomodRule: (serverId: string, name: string, ruleType: string, config: string, actionType: string, timeoutSeconds?: number) => Promise<void>;
  updateAutomodRule: (serverId: string, ruleId: string, name: string, enabled: boolean, config: string, actionType: string, timeoutSeconds?: number) => Promise<void>;
  deleteAutomodRule: (serverId: string, ruleId: string) => Promise<void>;
  listAutomodRules: (serverId: string) => void;
  // ── Phase 7: Community & Discovery ──
  createInvite: (serverId: string, maxUses?: number, expiresAt?: string, channelId?: string) => Promise<void>;
  listInvites: (serverId: string) => void;
  deleteInvite: (serverId: string, inviteId: string) => Promise<void>;
  useInvite: (code: string) => Promise<void>;
  createEvent: (serverId: string, name: string, startTime: string, options?: { description?: string; channelId?: string; endTime?: string; imageUrl?: string }) => Promise<void>;
  listEvents: (serverId: string) => void;
  updateEventStatus: (serverId: string, eventId: string, status: string) => Promise<void>;
  deleteEvent: (serverId: string, eventId: string) => Promise<void>;
  setRsvp: (serverId: string, eventId: string, status: string) => Promise<void>;
  removeRsvp: (serverId: string, eventId: string) => Promise<void>;
  listRsvps: (eventId: string) => void;
  updateCommunitySettings: (serverId: string, settings: { description?: string; isDiscoverable: boolean; welcomeMessage?: string; rulesText?: string; category?: string }) => Promise<void>;
  getCommunitySettings: (serverId: string) => void;
  discoverServers: (category?: string) => void;
  acceptRules: (serverId: string) => Promise<void>;
  setAnnouncementChannel: (serverId: string, channel: string, isAnnouncement: boolean) => Promise<void>;
  followChannel: (sourceChannelId: string, targetChannelId: string) => Promise<void>;
  unfollowChannel: (followId: string) => Promise<void>;
  listChannelFollows: (channelId: string) => void;
  createTemplate: (serverId: string, name: string, description?: string) => Promise<void>;
  listTemplates: (serverId: string) => void;
  deleteTemplate: (serverId: string, templateId: string) => Promise<void>;
  instantiateTemplate: (templateId: string, serverName: string) => Promise<void>;
  // ── Phase 8: Integrations & Bots ──
  createWebhook: (serverId: string, channelId: string, name: string, webhookType: string, url?: string) => void;
  listWebhooks: (serverId: string) => void;
  updateWebhook: (webhookId: string, name: string, avatarUrl?: string) => void;
  deleteWebhook: (webhookId: string) => void;
  createBot: (username: string) => void;
  listOwnedBots: () => void;
  clearBotCredential: () => void;
  createBotToken: (botUserId: string, name?: string, scopes?: string) => void;
  listBotTokens: (botUserId: string) => void;
  deleteBotToken: (tokenId: string) => void;
  addBotToServer: (botUserId: string, serverId: string) => void;
  removeBotFromServer: (botUserId: string, serverId: string) => void;
  registerSlashCommand: (serverId: string, name: string, description: string, optionsJson?: string) => void;
  listSlashCommands: (serverId: string) => void;
  deleteSlashCommand: (commandId: string) => void;
  invokeSlashCommand: (serverId: string, channelId: string, commandName: string, argsJson?: string) => Promise<void>;
  invokeMessageComponent: (messageId: string, customId: string, values?: string[]) => Promise<void>;
  createOAuth2App: (name: string, description: string, redirectUris: string, clientType: 'confidential' | 'public') => void;
  listOAuth2Apps: () => void;
  deleteOAuth2App: (appId: string) => void;
  // ── Phase 9.5: Premium-for-Free Features ──
  loadServerStickers: (serverId: string) => void;
  createSticker: (serverId: string, name: string, imageUrl: string, description?: string) => Promise<void>;
  deleteSticker: (serverId: string, stickerId: string) => Promise<void>;
  loadAllUserEmoji: (targetServerId: string) => void;
  setServerAvatar: (serverId: string, avatarUrl?: string | null) => void;
  setVanityCode: (serverId: string, vanityCode?: string | null) => Promise<void>;
  fetchServerLimits: () => void;
  // ── Phase 9: AT Protocol Deep Integration ──
  syncBlueskyProfile: () => Promise<void>;
  fetchBlueskyIdentity: (userId: string) => void;
  shareToBluesky: (messageId: string) => Promise<BlueskyShareResult>;
  fetchAtprotoSyncSetting: () => void;
  setAtprotoSyncEnabled: (enabled: boolean) => Promise<void>;
}

function protectedStateReset(): Partial<ChatState> {
  return {
    servers: EMPTY_SERVERS,
    channels: EMPTY_CHANNELS_MAP,
    messages: EMPTY_MESSAGES_MAP,
    members: EMPTY_MEMBERS_MAP,
    hasMore: EMPTY_HAS_MORE,
    avatars: EMPTY_AVATARS,
    typingUsers: EMPTY_TYPING,
    ownPresenceStatus: null,
    ownRequestedStatus: null,
    ownCustomStatus: null,
    ownStatusEmoji: null,
    unreadCounts: EMPTY_UNREAD,
    readSequences: EMPTY_READ_SEQUENCES,
    customEmoji: EMPTY_EMOJI,
    roles: EMPTY_ROLES,
    categories: EMPTY_CATEGORIES,
    presences: EMPTY_PRESENCES,
    userProfiles: EMPTY_PROFILES,
    searchResults: null,
    searchQuery: '',
    searchTotalCount: 0,
    searchOffset: 0,
    searchNextContinuation: null,
    searchContinuationTokens: {},
    activeSearchRequestId: null,
    searchRestarted: false,
    deletedMessageIds: {},
    pinnedMessages: EMPTY_PINS,
    threads: EMPTY_THREADS,
    forumTags: EMPTY_FORUM_TAGS,
    bookmarks: EMPTY_BOOKMARKS,
    notificationSettings: EMPTY_NOTIFICATION_SETTINGS,
    memberRoles: EMPTY_MEMBER_ROLES,
    channelPermissionOverrides: EMPTY_CHANNEL_OVERRIDES,
    directConversations: EMPTY_DIRECT_CONVERSATIONS,
    auditLog: {},
    bans: {},
    automodRules: {},
    invites: EMPTY_INVITES,
    serverEvents: EMPTY_EVENTS,
    eventRsvps: {},
    communitySettings: EMPTY_COMMUNITY,
    discoverableServers: EMPTY_DISCOVER,
    channelFollows: {},
    templates: EMPTY_TEMPLATES,
    webhooks: EMPTY_WEBHOOKS,
    slashCommands: EMPTY_SLASH_COMMANDS,
    botTokens: EMPTY_BOT_TOKENS,
    botAccounts: [],
    botCredential: null,
    oauth2Apps: EMPTY_OAUTH2_APPS,
    blueskyIdentities: EMPTY_BLUESKY_IDENTITIES,
    atprotoSyncEnabled: false,
    stickers: {},
    allUserEmoji: [],
    serverAvatars: {},
    maxMessageLength: 4000,
    maxFileSizeMb: 100,
  };
}

/** Cache an avatar_url for a nickname if present. */
function cacheAvatar(avatars: Record<string, string>, nickname: string, avatar_url?: string | null): Record<string, string> {
  if (avatar_url && avatars[nickname] !== avatar_url) {
    return { ...avatars, [nickname]: avatar_url };
  }
  return avatars;
}

function conversationKey(
  channels: Record<string, ChannelInfo[]>,
  directConversations: DirectConversationInfo[],
  conversationId: string,
): string | null {
  for (const [serverId, entries] of Object.entries(channels)) {
    const channel = entries.find((candidate) => candidate.conversation_id === conversationId);
    if (channel) return channelKey(serverId, channel.name);
  }
  if (directConversations.some((conversation) => conversation.id === conversationId)) {
    return `dm:${conversationId}`;
  }
  return null;
}

function entityVersionKey(entityType: string, entityId: string): string {
  return `${entityType}:${entityId}`;
}

function notificationPolicy(settings: NotificationSettingInfo[], channelId?: string) {
  const global = settings.find((setting) => !setting.server_id && !setting.channel_id);
  const server = settings.find((setting) => setting.server_id && !setting.channel_id);
  const channel = channelId ? settings.find((setting) => setting.channel_id === channelId) : undefined;
  const levels = [channel?.level, server?.level, global?.level];
  const level = levels.find((candidate) => candidate && candidate !== 'default') ?? 'mentions';
  const muteRows = [global, server, channel].filter((row): row is NotificationSettingInfo => Boolean(row));
  const muted = muteRows.some((row) => row.muted && (!row.mute_until || Date.parse(row.mute_until) > Date.now()));
  const controls = channel ?? server ?? global;
  return {
    level,
    muted,
    suppressEveryone: controls?.suppress_everyone ?? false,
    suppressRoles: controls?.suppress_roles ?? false,
  };
}

async function claimDesktopNotification(accountId: string, messageId: string): Promise<boolean> {
  if (typeof navigator === 'undefined' || !navigator.locks) return false;
  const ledgerKey = `concord:notification-ledger:${accountId}`;
  return navigator.locks.request(`concord:notification-lock:${accountId}:${messageId}`, async () => {
    try {
      const now = Date.now();
      const parsed = JSON.parse(localStorage.getItem(ledgerKey) ?? '{}') as unknown;
      const ledger: Record<string, number> = parsed && typeof parsed === 'object'
        ? Object.fromEntries(Object.entries(parsed).filter((entry): entry is [string, number] =>
            typeof entry[1] === 'number' && Number.isFinite(entry[1]) && now - entry[1] < 86_400_000))
        : {};
      if (ledger[messageId] !== undefined) return false;
      ledger[messageId] = now;
      const bounded = Object.fromEntries(Object.entries(ledger)
        .sort((left, right) => right[1] - left[1])
        .slice(0, 512));
      localStorage.setItem(ledgerKey, JSON.stringify(bounded));
      return true;
    } catch {
      // Without an atomic durable claim, suppress the alert instead of risking
      // duplicate notifications across tabs.
      return false;
    }
  });
}

function removeDeletedReferences(state: ChatState, deleted: Set<string>) {
  return {
    pinnedMessages: Object.fromEntries(Object.entries(state.pinnedMessages).map(([key, pins]) => [
      key, pins.filter((pin) => !deleted.has(pin.message_id)),
    ])),
    bookmarks: state.bookmarks.filter((bookmark) => !deleted.has(bookmark.message_id)),
    searchResults: state.searchResults?.filter((result) => !deleted.has(result.id)) ?? null,
  };
}

function withoutServers<T>(record: Record<string, T>, retained: Set<string>): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([serverId]) => retained.has(serverId)));
}

function withoutChannelKeys<T>(record: Record<string, T>, removed: Set<string>): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([key]) => {
    const separator = key.indexOf(':');
    return separator < 0 || !removed.has(key.slice(0, separator));
  }));
}

function redactDeletedReplyPreviews(messages: HistoryMessage[], deleted: Set<string>) {
  return messages.map((message) => message.reply_to && deleted.has(message.reply_to.id)
    ? { ...message, reply_to: { id: message.reply_to.id, from: message.reply_to.from, content_preview: '' } }
    : message);
}

async function maybeNotifyMessage(
  state: ChatState,
  message: {
    id: string;
    senderId?: string;
    senderNick: string;
    content?: string | null;
    mentions?: Array<{ kind: string; target_id?: string | null }>;
  },
  serverId?: string,
  channelId?: string,
): Promise<void> {
  if (typeof document === 'undefined' || document.visibilityState === 'visible') return;
  if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
  if (!state.activeAccountId || message.senderId === state.activeAccountId || message.senderNick === state.nickname) return;
  const isDnd = state.ownPresenceStatus === 'dnd'
    || Object.values(state.presences).some((server) => server[state.activeAccountId!]?.status === 'dnd');
  if (isDnd) return;
  const settings = serverId
    ? state.notificationSettings[serverId] ?? []
    : Object.values(state.notificationSettings).flat();
  const policy = notificationPolicy(settings, channelId);
  if (policy.muted || policy.level === 'none') return;
  const mentions = message.mentions ?? [];
  const mentionsMe = mentions.some((mention) => mention.kind === 'user' && mention.target_id === state.activeAccountId)
    || (!policy.suppressEveryone && mentions.some((mention) => mention.kind === 'everyone'))
    || (!policy.suppressRoles && serverId !== undefined && mentions.some((mention) =>
      mention.kind === 'role'
      && mention.target_id !== undefined
      && mention.target_id !== null
      && (state.memberRoles[serverId]?.[state.activeAccountId!] ?? []).includes(mention.target_id)));
  if (policy.level === 'mentions' && !mentionsMe) return;
  const accountId = state.activeAccountId;
  const protectedGeneration = state.protectedGeneration;
  if (!await claimDesktopNotification(accountId, message.id)) return;
  const current = useChatStore.getState();
  if (current.activeAccountId !== accountId || current.protectedGeneration !== protectedGeneration) return;
  new Notification(message.senderNick, {
    body: message.content?.trim() || 'Sent an attachment',
    tag: `concord-message-${message.id}`,
  });
}

function durableMessage(message: DurableMessageProjection): HistoryMessage {
  return {
    id: message.message_id,
    from: message.sender_nick,
    sender_id: message.sender_id,
    sequence: message.sequence,
    deleted: message.deleted,
    content: message.deleted ? '' : (message.content ?? ''),
    timestamp: message.created_at,
    edited_at: message.edited_at,
    reply_to: message.reply_to ? {
      id: message.reply_to.message_id,
      from: message.reply_to.sender_nick,
      content_preview: message.reply_to.deleted ? '' : (message.reply_to.content ?? ''),
    } : null,
    attachments: message.deleted ? [] : message.attachments.map((attachment) => ({
      id: attachment.attachment_id,
      filename: attachment.filename,
      content_type: attachment.content_type,
      file_size: attachment.file_size,
      url: `/api/uploads/${encodeURIComponent(attachment.attachment_id)}`,
    })),
    rich_embeds: message.deleted ? null : message.rich_embeds,
    components: message.deleted ? null : message.components,
  };
}

function mergeDurableMessage(current: HistoryMessage[], projection: DurableMessageProjection): HistoryMessage[] {
  const message = durableMessage(projection);
  const existing = current.find((candidate) => candidate.id === message.id);
  const merged = existing ? { ...existing, ...message, reactions: existing.reactions } : message;
  const next = [...current.filter((candidate) => candidate.id !== message.id), merged];
  next.sort((left, right) => left.timestamp.localeCompare(right.timestamp) || left.id.localeCompare(right.id));
  return next.length > MAX_MESSAGES_PER_CHANNEL ? next.slice(-MAX_MESSAGES_PER_CHANNEL) : next;
}

function applyReactionProjection(
  messages: HistoryMessage[],
  reaction: { message_id: string; emoji: string; present: boolean; user_id: string },
): HistoryMessage[] {
  return messages.map((message) => {
    if (message.id !== reaction.message_id) return message;
    const groups = [...(message.reactions ?? [])];
    const index = groups.findIndex((group) => group.emoji === reaction.emoji);
    if (reaction.present) {
      if (index < 0) groups.push({ emoji: reaction.emoji, count: 1, user_ids: [reaction.user_id] });
      else if (!groups[index].user_ids.includes(reaction.user_id)) {
        const user_ids = [...groups[index].user_ids, reaction.user_id];
        groups[index] = { ...groups[index], user_ids, count: Math.max(groups[index].count + 1, user_ids.length) };
      }
    } else if (index >= 0) {
      const user_ids = groups[index].user_ids.filter((id) => id !== reaction.user_id);
      const count = Math.max(0, groups[index].count - 1);
      if (count === 0) groups.splice(index, 1);
      else groups[index] = { ...groups[index], user_ids, count };
    }
    return { ...message, reactions: groups };
  });
}

function snapshotReactions(messageId: string, reactions: SnapshotReactionGroup[]) {
  return reactions
    .filter((reaction) => reaction.message_id === messageId)
    .map((reaction) => ({
      emoji: reaction.emoji,
      count: reaction.count,
      user_ids: reaction.reacted_by_me ? ['__self__'] : [],
    }));
}

function currentSubscriptions(
  channels: Record<string, ChannelInfo[]>,
  directConversations: DirectConversationInfo[] = [],
): string[] {
  return [...new Set([
    ...Object.values(channels).flat().map((channel) => channel.conversation_id),
    ...directConversations.map((conversation) => conversation.id),
  ])].sort();
}

function requestSync(
  ws: WebSocketManager,
  subscriptions: string[],
  cursor?: string,
  windowCursors: Record<string, string> = {},
): void {
  const windows = subscriptions.length
    ? Array.from({ length: Math.ceil(subscriptions.length / 100) }, (_, index) =>
        subscriptions.slice(index * 100, (index + 1) * 100))
    : [[]];
  for (const window of windows) {
    const windowId = window.join('\u0000');
    const resumeCursor = cursor ?? windowCursors[windowId];
    const requestId = crypto.randomUUID();
    useConnectionStore.getState().registerSync(requestId, window);
    ws.send({
      type: 'sync',
      request_id: requestId,
      protocol_version: 2,
      subscriptions: window,
      ...(resumeCursor ? { cursor: resumeCursor } : {}),
      limit: 100,
    });
  }
}

function rejectPendingInteractions(reason: string) {
  usePendingStore.getState().rejectAllInteractions(reason);
  usePendingStore.getState().rejectAllLifecycle(UNCERTAIN_LIFECYCLE_MESSAGE);
}

function durableProjectionUpdate(
  projection: DurableEventProjection,
  state: ChatState,
): Partial<ChatState> {
  const key = conversationKey(state.channels, state.directConversations, projection.conversation_id);
  if (!key) return {};
  const versionKey = entityVersionKey(projection.entity_type, projection.entity_id);
  if ((state.entityVersions[versionKey] ?? 0) >= projection.entity_version) return {};
  const entityVersions = {
    ...state.entityVersions,
    [versionKey]: projection.entity_version,
  };
  if (projection.message) {
    const current = state.messages[key] ?? [];
    const isNewVisibleMessage = !projection.message.deleted
      && !current.some((message) => message.id === projection.message?.message_id)
      && projection.message.sender_id !== state.activeAccountId
      && BigInt(projection.message.sequence) > BigInt(state.readSequences[projection.conversation_id] ?? '0');
    const mergedMessages = mergeDurableMessage(current, projection.message);
    const deleted = new Set([projection.message.message_id]);
    const messages = projection.message.deleted
      ? redactDeletedReplyPreviews(mergedMessages, deleted)
      : mergedMessages;
    return {
      messages: {
        ...state.messages,
        [key]: messages,
      },
      unreadCounts: isNewVisibleMessage
        ? { ...state.unreadCounts, [key]: (state.unreadCounts[key] ?? 0) + 1 }
        : state.unreadCounts,
      entityVersions,
      ...(projection.message.deleted ? {
        ...removeDeletedReferences(state, deleted),
        deletedMessageIds: { ...state.deletedMessageIds, [projection.message.message_id]: true },
      } : {}),
    };
  }
  if (projection.reaction) {
    return {
      messages: {
        ...state.messages,
        [key]: applyReactionProjection(state.messages[key] ?? [], projection.reaction),
      },
      entityVersions,
    };
  }
  if (projection.read_state) {
    const previousSequence = state.readSequences[projection.conversation_id] ?? '0';
    if (BigInt(projection.read_state.sequence) <= BigInt(previousSequence)) {
      return { entityVersions };
    }
    const unread = (state.messages[key] ?? []).filter((message) =>
      !message.deleted
      && message.sender_id !== state.activeAccountId
      && BigInt(message.sequence ?? '0') > BigInt(projection.read_state!.sequence)).length;
    return {
      unreadCounts: { ...state.unreadCounts, [key]: unread },
      readSequences: {
        ...state.readSequences,
        [projection.conversation_id]: projection.read_state.sequence,
      },
      entityVersions,
    };
  }
  if (projection.entity_type === 'thread_state'
    && projection.kind === 'thread_state_changed'
    && typeof projection.descriptor === 'object'
    && projection.descriptor !== null
    && 'archived' in projection.descriptor
    && typeof projection.descriptor.archived === 'boolean') {
    const archived = projection.descriptor.archived;
    const channels = Object.fromEntries(Object.entries(state.channels).map(([serverId, entries]) => [
      serverId,
      entries.map((channel) => channel.id === projection.entity_id ? { ...channel, archived } : channel),
    ]));
    const threads = Object.fromEntries(Object.entries(state.threads).map(([parent, entries]) => [
      parent,
      entries.map((thread) => thread.id === projection.entity_id ? { ...thread, archived } : thread),
    ]));
    return { channels, threads, entityVersions };
  }
  if (projection.entity_type === 'thread_tags'
    && projection.kind === 'thread_tags_updated'
    && typeof projection.descriptor === 'object'
    && projection.descriptor !== null
    && 'thread_id' in projection.descriptor
    && typeof projection.descriptor.thread_id === 'string'
    && 'tag_ids' in projection.descriptor
    && Array.isArray(projection.descriptor.tag_ids)
    && projection.descriptor.tag_ids.every((tag): tag is string => typeof tag === 'string')) {
    const threadId = projection.descriptor.thread_id;
    const tagIds = [...projection.descriptor.tag_ids];
    const threads = Object.fromEntries(Object.entries(state.threads).map(([parent, entries]) => [
      parent,
      entries.map((thread) => thread.id === threadId
        ? { ...thread, tag_ids: tagIds, tags_version: projection.entity_version }
        : thread),
    ]));
    return { threads, entityVersions };
  }
  return {};
}

const CONNECTION_KEYS = [
  'connected', 'ws', 'nickname', 'activeAccountId', 'accountGeneration',
  'protectedGeneration', 'operationGeneration', 'syncCursor',
  'syncWindowCursors', 'durableMode',
  'ownPresenceStatus',
  'ownRequestedStatus', 'ownCustomStatus', 'ownStatusEmoji',
] as const satisfies ReadonlyArray<keyof ConnectionState>;
let coordinatedDomainUpdate = false;

function updateDomainStores(next: Partial<ChatState>) {
  coordinatedDomainUpdate = true;
  try {
  const connection: Partial<Omit<ConnectionState, 'replace'>> = {};
  for (const key of CONNECTION_KEYS) {
    if (key in next) Object.assign(connection, { [key]: next[key as keyof ChatState] });
  }
  if (Object.keys(connection).length > 0) useConnectionStore.getState().replace(connection);
  if (next.pendingCommands !== undefined) usePendingStore.getState().replace(next.pendingCommands);
  const entities = {
    ...(next.servers !== undefined ? { servers: next.servers } : {}),
    ...(next.channels !== undefined ? { channels: next.channels } : {}),
    ...(next.messages !== undefined ? { messages: next.messages } : {}),
    ...(next.members !== undefined ? { members: next.members } : {}),
    ...(next.directConversations !== undefined ? { directConversations: next.directConversations } : {}),
    ...(next.entityVersions !== undefined ? { entityVersions: next.entityVersions } : {}),
    ...(next.deletedMessageIds !== undefined ? { deletedMessageIds: next.deletedMessageIds } : {}),
  };
  if (Object.keys(entities).length > 0) useEntityStore.getState().replace(entities);
  } finally {
    coordinatedDomainUpdate = false;
  }
}

export const useChatStore = create<ChatState>((rawSet, get) => {
  const set = ((next: ChatState | Partial<ChatState> | ((state: ChatState) => ChatState | Partial<ChatState>), replace?: boolean) => {
    const resolved = typeof next === 'function' ? next(get()) : next;
    updateDomainStores(resolved);
    if (replace === true) rawSet(resolved as ChatState, true);
    else rawSet(resolved);
  }) as typeof rawSet;
  return ({
  connected: false,
  operationGeneration: null,
  syncCursor: null,
  syncWindowCursors: {},
  pendingCommands: {},
  accountGeneration: 0,
  protectedGeneration: 0,
  durableMode: false,
  ownPresenceStatus: null,
  ownRequestedStatus: null,
  ownCustomStatus: null,
  ownStatusEmoji: null,
  entityVersions: {},
  readSequences: EMPTY_READ_SEQUENCES,
  drafts: {},
  compositionFiles: {},
  failedCompositions: [],
  directConversations: EMPTY_DIRECT_CONVERSATIONS,
  activeAccountId: null,
  nickname: null,
  servers: EMPTY_SERVERS,
  channels: EMPTY_CHANNELS_MAP,
  messages: EMPTY_MESSAGES_MAP,
  members: EMPTY_MEMBERS_MAP,
  hasMore: EMPTY_HAS_MORE,
  avatars: EMPTY_AVATARS,
  typingUsers: EMPTY_TYPING,
  replyingTo: null,
  unreadCounts: EMPTY_UNREAD,
  customEmoji: EMPTY_EMOJI,
  roles: EMPTY_ROLES,
  categories: EMPTY_CATEGORIES,
  presences: EMPTY_PRESENCES,
  userProfiles: EMPTY_PROFILES,
  searchResults: null,
  searchQuery: '',
  searchTotalCount: 0,
  searchOffset: 0,
  searchNextContinuation: null,
  searchContinuationTokens: {},
  activeSearchRequestId: null,
  searchRestarted: false,
  deletedMessageIds: {},
  pinnedMessages: EMPTY_PINS,
  threads: EMPTY_THREADS,
  forumTags: EMPTY_FORUM_TAGS,
  bookmarks: EMPTY_BOOKMARKS,
  notificationSettings: EMPTY_NOTIFICATION_SETTINGS,
  memberRoles: EMPTY_MEMBER_ROLES,
  roleProjectionVersions: {},
  channelPermissionOverrides: EMPTY_CHANNEL_OVERRIDES,
  auditLog: {} as Record<string, AuditLogEntry[]>,
  bans: {} as Record<string, BanInfo[]>,
  automodRules: {} as Record<string, AutomodRuleInfo[]>,
  invites: EMPTY_INVITES,
  serverEvents: EMPTY_EVENTS,
  eventRsvps: {},
  communitySettings: EMPTY_COMMUNITY,
  discoverableServers: EMPTY_DISCOVER,
  channelFollows: {},
  templates: EMPTY_TEMPLATES,
  webhooks: EMPTY_WEBHOOKS,
  slashCommands: EMPTY_SLASH_COMMANDS,
  botTokens: EMPTY_BOT_TOKENS,
  botAccounts: [],
  botCredential: null,
  oauth2Apps: EMPTY_OAUTH2_APPS,
  blueskyIdentities: EMPTY_BLUESKY_IDENTITIES,
  atprotoSyncEnabled: false,
  stickers: {} as Record<string, StickerInfo[]>,
  allUserEmoji: [],
  serverAvatars: {} as Record<string, Record<string, string>>,
  maxMessageLength: 4000,
  maxFileSizeMb: 100,
  errorToast: null,
  ws: null,

  sendTracked: (requestId, command) => {
    const ws = get().ws;
    if (!ws) return false;
    pendingCommandOwners.set(requestId, get().accountGeneration);
    set((state) => ({ pendingCommands: { ...state.pendingCommands, [requestId]: command } }));
    if (ws.send(command)) return true;
    set((state) => {
      const pendingCommands = { ...state.pendingCommands };
      delete pendingCommands[requestId];
      pendingCommandOwners.delete(requestId);
      return { pendingCommands };
    });
    return false;
  },

  runLifecycleCommand: (command, pendingKey) => {
    const ws = get().ws;
    if (!ws || !get().connected) return Promise.reject(new Error('Not connected.'));
    const requestId = crypto.randomUUID();
    const accountGeneration = get().accountGeneration;
    const scopedPendingKey = `${get().activeAccountId ?? ''}\u0000${pendingKey}`;
    const result = new Promise<void>((resolve, reject) => {
      const deadline = setTimeout(() => {
        usePendingStore.getState().takeLifecycle(requestId)?.reject(
          new LifecycleOutcomeUncertainError(
            'The action result timed out and is unknown; refresh current state before retrying.',
          ),
        );
      }, LIFECYCLE_RESULT_TIMEOUT_MS);
      const registered = usePendingStore.getState().registerLifecycle(requestId, {
        accountGeneration,
        connection: ws,
        key: scopedPendingKey,
        deadline,
        resolve,
        reject,
      });
      if (!registered) {
        clearTimeout(deadline);
        reject(new Error('This action is already pending or the pending action limit was reached.'));
      }
    });
    const pending = usePendingStore.getState().lifecycleCommands[requestId];
    if (!pending) return result;
    try {
      if (ws.send({ type: 'lifecycle_command', request_id: requestId, command })) return result;
    } catch (error) {
      usePendingStore.getState().takeLifecycle(requestId)?.reject(
        error instanceof Error ? error : new Error('Action was not sent.'),
      );
      return result;
    }
    usePendingStore.getState().takeLifecycle(requestId)?.reject(
      new Error('Action was not sent; reconnecting.'),
    );
    return result;
  },

  setDraft: (key, text) => useComposerStore.getState().setDraft(key, text),

  setCompositionFiles: (key, files) => useComposerStore.getState().setCompositionFiles(key, files),

  retryFailedComposition: (id) => {
    const failed = useComposerStore.getState().failedCompositions.find((entry) => entry.id === id);
    if (!failed || failed.accountId !== get().activeAccountId) return false;
    const previousReply = useComposerStore.getState().replies[failed.key] ?? null;
    useComposerStore.getState().setReplyFor(failed.key, failed.replyTo);
    const accepted = failed.conversationId && failed.recipient
      ? get().sendDirectMessage(failed.conversationId, failed.recipient, failed.content, failed.attachments)
      : failed.serverId !== null && get().sendMessage(
          failed.serverId, failed.channel, failed.content, failed.attachments,
        );
    useComposerStore.getState().setReplyFor(failed.key, previousReply);
    if (accepted) {
      useComposerStore.getState().setFailedCompositions(
        useComposerStore.getState().failedCompositions.filter((entry) => entry.id !== id),
      );
    }
    return accepted;
  },

  dismissFailedComposition: (id) => useComposerStore.getState().setFailedCompositions(
    useComposerStore.getState().failedCompositions.filter((entry) => entry.id !== id),
  ),

  connect: (nickname: string, accountId = nickname) => {
    if (get().ws) {
      return;
    }

    const accountGeneration = get().accountGeneration + 1;
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${protocol}//${window.location.host}/ws?nickname=${encodeURIComponent(nickname)}`;

    const ws = new WebSocketManager(
      url,
      (event) => {
        get().handleEvent(event);
      },
      (connected) => {
        set({ connected });
        if (!connected) {
          recoveredThisConnection.clear();
          hydratedServerMetadata.clear();
          useConnectionStore.getState().clearSync();
          rejectPendingInteractions('Disconnected before the interaction completed.');
          useComposerStore.getState().clearReplies();
          set({
            ...protectedStateReset(),
            protectedGeneration: get().protectedGeneration + 1,
            operationGeneration: null,
            syncCursor: null,
            syncWindowCursors: {},
            durableMode: false,
            entityVersions: {},
          });
        }
        if (connected) {
          // Clear stale ephemeral state on reconnect
          set({ typingUsers: EMPTY_TYPING });
          useComposerStore.getState().clearReplies();
          ws.send({ type: 'list_servers' });
          ws.send({ type: 'list_direct_conversations' });
          ws.send({ type: 'get_server_limits' });
          requestSync(ws, []);
        }
      },
    );

    useComposerStore.getState().replaceState({
      drafts: { ...(draftsByAccount.get(accountId) ?? {}) },
      compositionFiles: {},
      failedCompositions: [],
      replyingTo: null,
    });
    set({
      ws,
      nickname,
      activeAccountId: accountId,
      accountGeneration,
      protectedGeneration: get().protectedGeneration + 1,
    });
    ws.connect();
  },

  disconnect: () => {
    const { activeAccountId } = get();
    const { drafts } = useComposerStore.getState();
    if (activeAccountId) draftsByAccount.set(activeAccountId, { ...drafts });
    get().ws?.disconnect();
    for (const timer of retryTimers.values()) clearTimeout(timer);
    retryTimers.clear();
    pendingCommandOwners.clear();
    retriedCommands.clear();
    recoveredThisConnection.clear();
    hydratedServerMetadata.clear();
    useConnectionStore.getState().clearSync();
    rejectPendingInteractions('Disconnected before the interaction completed.');
    useComposerStore.getState().replaceState({
      drafts: {}, compositionFiles: {}, failedCompositions: [], replyingTo: null,
    });
    set({
      ...protectedStateReset(),
      ws: null,
      connected: false,
      operationGeneration: null,
      syncCursor: null,
      syncWindowCursors: {},
      pendingCommands: {},
      accountGeneration: get().accountGeneration + 1,
      protectedGeneration: get().protectedGeneration + 1,
      durableMode: false,
      entityVersions: {},
      activeAccountId: null,
    });
  },

  handleEvent: (event: ServerEvent) => {
    switch (event.type) {
      case 'sync_snapshot': {
        const syncSubscriptions = useConnectionStore.getState().syncSubscriptions;
        const correlated = syncSubscriptions.get(event.request_id);
        if (!correlated && (get().durableMode || syncSubscriptions.size > 0)) break;
        const subscriptions = correlated ?? [];
        const windowId = subscriptions.join('\u0000');
        set((state) => {
          const replacedKeys = new Set(subscriptions
            .map((conversation) => conversationKey(state.channels, state.directConversations, conversation))
            .filter((key): key is string => key !== null));
          const replacedMessageIds = new Set(Object.entries(state.messages)
            .filter(([key]) => replacedKeys.has(key))
            .flatMap(([, entries]) => entries.map((message) => message.id)));
          const messages: Record<string, HistoryMessage[]> = Object.fromEntries(
            Object.entries(state.messages).filter(([key]) => !replacedKeys.has(key)),
          );
          const unreadCounts: Record<string, number> = Object.fromEntries(
            Object.entries(state.unreadCounts).filter(([key]) => !replacedKeys.has(key)),
          );
          const hasMore: Record<string, boolean> = Object.fromEntries(
            Object.entries(state.hasMore).filter(([key]) => !replacedKeys.has(key)),
          );
          const entityVersions: Record<string, number> = Object.fromEntries(
            Object.entries(state.entityVersions).filter(([entity]) =>
              ![...replacedMessageIds].some((messageId) => entity === entityVersionKey('message', messageId))
              && !subscriptions.some((conversation) => entity === entityVersionKey('read_state', conversation))),
          );
          const snapshotReadSequences = new Map(
            event.snapshot.read_states.map((read) => [read.conversation_id, BigInt(read.sequence)]),
          );
          const readSequences = Object.fromEntries(Object.entries(state.readSequences)
            .filter(([conversation]) => !subscriptions.includes(conversation)));
          for (const projection of event.snapshot.messages) {
            entityVersions[entityVersionKey('message', projection.message_id)] = projection.entity_version;
            const key = conversationKey(state.channels, state.directConversations, projection.conversation_id);
            if (!key) continue;
            const mapped = durableMessage(projection);
            mapped.reactions = snapshotReactions(projection.message_id, event.snapshot.reactions);
            messages[key] = [...(messages[key] ?? []), mapped];
            if (!projection.deleted
                && projection.sender_id !== state.activeAccountId
                && BigInt(projection.sequence) > (snapshotReadSequences.get(projection.conversation_id) ?? 0n)) {
              unreadCounts[key] = (unreadCounts[key] ?? 0) + 1;
            }
            hasMore[key] = event.snapshot.history_before[projection.conversation_id] !== undefined;
          }
          for (const read of event.snapshot.read_states) {
            entityVersions[entityVersionKey('read_state', read.conversation_id)] = read.entity_version;
            readSequences[read.conversation_id] = read.sequence;
          }
          return {
            operationGeneration: event.snapshot.operation_generation,
            syncCursor: event.snapshot.cursor,
            syncWindowCursors: { ...state.syncWindowCursors, [windowId]: event.snapshot.cursor },
            messages,
            unreadCounts,
            hasMore,
            durableMode: true,
            entityVersions,
            readSequences,
          };
        });
        useConnectionStore.getState().removeSync(event.request_id);
        const { accountGeneration, pendingCommands, ws } = get();
        let recovered = 0;
        for (const [requestId, command] of Object.entries(pendingCommands)) {
          if (pendingCommandOwners.get(requestId) === accountGeneration
              && !recoveredThisConnection.has(requestId)
              && ws?.send(command)) {
            recoveredThisConnection.add(requestId);
            recovered += 1;
          }
        }
        if (recovered > 0) {
          set({ errorToast: `Restoring ${recovered} pending ${recovered === 1 ? 'change' : 'changes'}…` });
          setTimeout(() => set({ errorToast: null }), 5000);
        }
        break;
      }

      case 'replay_batch': {
        const correlated = useConnectionStore.getState().syncSubscriptions.get(event.request_id);
        if (!correlated) break;
        const subscriptions = correlated;
        const windowId = subscriptions.join('\u0000');
        for (const projection of event.batch.events) set((state) => durableProjectionUpdate(projection, state));
        set((state) => ({
          operationGeneration: event.batch.operation_generation,
          syncCursor: event.batch.cursor,
          syncWindowCursors: { ...state.syncWindowCursors, [windowId]: event.batch.cursor },
        }));
        if (event.batch.has_more) {
          useConnectionStore.getState().removeSync(event.request_id);
          if (get().ws) requestSync(get().ws!, subscriptions, event.batch.cursor, get().syncWindowCursors);
        } else useConnectionStore.getState().removeSync(event.request_id);
        break;
      }

      case 'durable_event': {
        if (event.event.message && !event.event.message.deleted) {
          const state = get();
          const key = conversationKey(state.channels, state.directConversations, event.event.conversation_id);
          const isNew = key && !(state.messages[key] ?? []).some((message) => message.id === event.event.message?.message_id);
          if (isNew) {
            const channelEntry = Object.entries(state.channels)
              .flatMap(([serverId, channels]) => channels.map((channel) => ({ serverId, channel })))
              .find(({ channel }) => channel.conversation_id === event.event.conversation_id);
            maybeNotifyMessage(state, {
              id: event.event.message.message_id,
              senderId: event.event.message.sender_id,
              senderNick: event.event.message.sender_nick,
              content: event.event.message.content,
              mentions: event.event.message.mentions,
            }, channelEntry?.serverId, channelEntry?.channel.id);
          }
        }
        set((state) => durableProjectionUpdate(event.event, state));
        break;
      }

      case 'resync_required': {
        const serverIds = get().servers.map((server) => server.id);
        hydratedServerMetadata.clear();
        useConnectionStore.getState().clearSync();
        useComposerStore.getState().setReplyingTo(null);
        set({
          ...protectedStateReset(),
          protectedGeneration: get().protectedGeneration + 1,
          operationGeneration: null,
          syncCursor: null,
          syncWindowCursors: {},
          durableMode: true,
          entityVersions: {},
        });
        get().ws?.send({ type: 'list_servers' });
        for (const serverId of serverIds) get().ws?.send({ type: 'list_channels', server_id: serverId });
        if (get().ws) requestSync(get().ws!, []);
        break;
      }

      case 'command_error': {
        const interaction = usePendingStore.getState().takeInteraction(event.request_id);
        if (interaction) {
          interaction.reject(new Error(event.message));
        }
        const lifecycle = usePendingStore.getState().takeLifecycle(event.request_id);
        if (lifecycle) lifecycle.reject(new Error(event.message));
        if (event.code === 'OPERATION_GENERATION_EXPIRED') {
          retriedCommands.delete(event.request_id);
          const timer = retryTimers.get(event.request_id);
          if (timer) clearTimeout(timer);
          retryTimers.delete(event.request_id);
          pendingCommandOwners.delete(event.request_id);
          set((state) => {
            const pendingCommands = { ...state.pendingCommands };
            delete pendingCommands[event.request_id];
            return { operationGeneration: null, syncCursor: null, pendingCommands };
          });
          get().ws?.send({
            type: 'sync',
            request_id: crypto.randomUUID(),
            protocol_version: 2,
            subscriptions: [],
            limit: 100,
          });
        } else if (event.retryable) {
          if (!retriedCommands.has(event.request_id)) {
            const pending = get().pendingCommands[event.request_id];
            const scheduledWs = get().ws;
            const scheduledGeneration = get().accountGeneration;
            if (pending
                && scheduledWs
                && pendingCommandOwners.get(event.request_id) === scheduledGeneration) {
              retriedCommands.add(event.request_id);
              const retryDelayMs = event.code === 'RATE_LIMITED' ? 1_100 : 250;
              const timer = setTimeout(() => {
                retryTimers.delete(event.request_id);
                if (get().accountGeneration === scheduledGeneration && get().ws === scheduledWs) {
                  scheduledWs.send(pending);
                }
              }, retryDelayMs);
              retryTimers.set(event.request_id, timer);
            }
          } else {
            set({ errorToast: `${event.message} The change is still pending.` });
          }
        } else {
          const rejected = get().pendingCommands[event.request_id];
          const rejectedOwner = pendingCommandOwners.get(event.request_id);
          retriedCommands.delete(event.request_id);
          const timer = retryTimers.get(event.request_id);
          if (timer) clearTimeout(timer);
          retryTimers.delete(event.request_id);
          pendingCommandOwners.delete(event.request_id);
          if (rejected?.type === 'send_message' || rejected?.type === 'send_direct_message') {
            const state = get();
            const direct = rejected.type === 'send_direct_message';
            const conversationId = direct
              ? state.directConversations.find((conversation) => conversation.peer_username.toLowerCase() === rejected.recipient.toLowerCase())?.id
              : undefined;
            if ((!direct && rejected.server_id) || (direct && conversationId)) {
              const key = direct ? `dm:${conversationId}` : channelKey(rejected.server_id!, rejected.channel);
              const optimistic = (state.messages[key] ?? []).find((message) => message.id === event.request_id);
              const composer = useComposerStore.getState();
              if (rejectedOwner === state.accountGeneration && !composer.drafts[key]) {
                composer.setDraft(key, rejected.content);
              }
              if (rejectedOwner === state.accountGeneration && state.activeAccountId) {
                composer.setFailedCompositions([...useComposerStore.getState().failedCompositions, {
                  id: event.request_id,
                  accountId: state.activeAccountId,
                  serverId: direct ? null : rejected.server_id!,
                  channel: direct ? rejected.recipient : rejected.channel,
                  ...(direct ? { conversationId, recipient: rejected.recipient } : {}),
                  key,
                  content: rejected.content,
                  attachments: optimistic?.attachments ?? [],
                  replyTo: optimistic?.reply_to ?? null,
                  error: event.message,
                }]);
              }
            }
          }
          set((state) => {
            const pendingCommands = { ...state.pendingCommands };
            delete pendingCommands[event.request_id];
            if (rejected?.type !== 'send_message' && rejected?.type !== 'send_direct_message') return { pendingCommands };
            const direct = rejected.type === 'send_direct_message';
            const conversationId = direct
              ? state.directConversations.find((conversation) => conversation.peer_username.toLowerCase() === rejected.recipient.toLowerCase())?.id
              : undefined;
            if (direct && !conversationId) return { pendingCommands };
            if (!direct && !rejected.server_id) return { pendingCommands };
            const key = direct ? `dm:${conversationId}` : channelKey(rejected.server_id!, rejected.channel);
            const messages = {
              ...state.messages,
              [key]: (state.messages[key] ?? []).filter((message) => message.id !== event.request_id),
            };
            return { pendingCommands, messages };
          });
        }
        set({ errorToast: event.message });
        setTimeout(() => set({ errorToast: null }), 5000);
        break;
      }

      case 'command_committed': {
        retriedCommands.delete(event.receipt.request_id);
        retriedCommands.delete(event.receipt.client_message_id);
        for (const requestId of [event.receipt.request_id, event.receipt.client_message_id]) {
          const timer = retryTimers.get(requestId);
          if (timer) clearTimeout(timer);
          retryTimers.delete(requestId);
          pendingCommandOwners.delete(requestId);
          recoveredThisConnection.delete(requestId);
        }
        set((state) => {
          const pendingCommands = { ...state.pendingCommands };
          delete pendingCommands[event.receipt.request_id];
          delete pendingCommands[event.receipt.client_message_id];
          const messages = Object.fromEntries(
            Object.entries(state.messages).map(([key, entries]) => [
              key,
              entries.map((message) => message.id === event.receipt.client_message_id
                ? { ...message, id: event.receipt.message_id }
                : message),
            ]),
          );
          return { pendingCommands, messages };
        });
        break;
      }

      case 'message': {
        // Canonical DM fanout currently arrives as a guarded Message event; channel
        // messages are projected through protocol-v2 durable events.
        if (get().durableMode && !event.conversation_id) break;
        const key = event.conversation_id
          ? `dm:${event.conversation_id}`
          : channelKey(event.server_id || 'default', event.target);
        const msg: HistoryMessage = {
          id: event.id,
          from: event.from,
          content: event.content,
          timestamp: event.timestamp,
          reply_to: event.reply_to,
          attachments: event.attachments,
        };
        maybeNotifyMessage(get(), {
          id: event.id,
          senderNick: event.from,
          content: event.content,
        }, event.server_id ?? undefined, get().channels[event.server_id || '']?.find((channel) => channel.name === event.target)?.id);
        set((s) => {
          if (s.entityVersions[entityVersionKey('message', event.id)] !== undefined
              || (s.messages[key] ?? []).some((message) => message.id === event.id)) return {};
          // Increment unread count for messages from others
          const newUnread = { ...s.unreadCounts };
          if (event.from !== s.nickname) {
            newUnread[key] = (newUnread[key] || 0) + 1;
          }
          const updated = [...(s.messages[key] || []), msg];
          const trimmed = updated.length > MAX_MESSAGES_PER_CHANNEL
            ? updated.slice(updated.length - MAX_MESSAGES_PER_CHANNEL)
            : updated;
          return {
            messages: {
              ...s.messages,
              [key]: trimmed,
            },
            avatars: cacheAvatar(s.avatars, event.from, event.avatar_url),
            unreadCounts: newUnread,
          };
        });
        break;
      }

      case 'message_edit': {
        if (get().durableMode) break;
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: (s.messages[key] || []).map((m) =>
              m.id === event.id ? { ...m, content: event.content, edited_at: event.edited_at } : m,
            ),
          },
        }));
        break;
      }

      case 'message_delete': {
        if (get().durableMode) break;
        const key = channelKey(event.server_id, event.channel);
        const deleted = new Set([event.id]);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: redactDeletedReplyPreviews(
              (s.messages[key] || []).filter((message) => !deleted.has(message.id)), deleted,
            ),
          },
          ...removeDeletedReferences(s, deleted),
          deletedMessageIds: { ...s.deletedMessageIds, [event.id]: true },
        }));
        break;
      }

      case 'message_ack': {
        // Server sends back the real message ID + nonce — update the optimistic local message
        const ackKey = event.conversation_id
          ? `dm:${event.conversation_id}`
          : channelKey(event.server_id, event.channel);
        if (!event.nonce) break;
        set((s) => {
          const msgs = s.messages[ackKey];
          if (!msgs) return {};
          // Find the optimistic message by its nonce (used as the temporary ID)
          const idx = msgs.findIndex((m) => m.id === event.nonce);
          if (idx === -1) return {};
          const updated = [...msgs];
          updated[idx] = { ...updated[idx], id: event.id };
          return { messages: { ...s.messages, [ackKey]: updated } };
        });
        break;
      }

      case 'message_embed': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: (s.messages[key] || []).map((m) =>
              m.id === event.message_id ? { ...m, embeds: event.embeds } : m,
            ),
          },
        }));
        break;
      }

      case 'reaction_add': {
        if (get().durableMode) break;
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: (s.messages[key] || []).map((m) => {
              if (m.id !== event.message_id) return m;
              const reactions = [...(m.reactions || [])];
              const existing = reactions.find((r) => r.emoji === event.emoji);
              if (existing) {
                if (!existing.user_ids.includes(event.user_id)) {
                  existing.user_ids = [...existing.user_ids, event.user_id];
                  existing.count = existing.user_ids.length;
                }
              } else {
                reactions.push({ emoji: event.emoji, count: 1, user_ids: [event.user_id] });
              }
              return { ...m, reactions };
            }),
          },
        }));
        break;
      }

      case 'reaction_remove': {
        if (get().durableMode) break;
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: (s.messages[key] || []).map((m) => {
              if (m.id !== event.message_id) return m;
              let reactions = (m.reactions || [])
                .map((r) => {
                  if (r.emoji !== event.emoji) return r;
                  const user_ids = r.user_ids.filter((uid) => uid !== event.user_id);
                  return { ...r, user_ids, count: user_ids.length };
                })
                .filter((r) => r.count > 0);
              if (reactions.length === 0) reactions = [];
              return { ...m, reactions };
            }),
          },
        }));
        break;
      }

      case 'typing_start': {
        const key = channelKey(event.server_id, event.channel);
        const myNick = get().nickname;
        if (event.nickname === myNick) break; // Don't show own typing
        set((s) => {
          const current = s.typingUsers[key] || [];
          if (current.includes(event.nickname)) return s;
          return {
            typingUsers: { ...s.typingUsers, [key]: [...current, event.nickname] },
          };
        });
        // Clear any previous timeout for this user+channel, then set a new one
        const timeoutKey = `${key}:${event.nickname}`;
        const prev = typingTimeouts.get(timeoutKey);
        if (prev) clearTimeout(prev);
        typingTimeouts.set(timeoutKey, setTimeout(() => {
          typingTimeouts.delete(timeoutKey);
          set((s) => {
            const current = s.typingUsers[key] || [];
            const filtered = current.filter((n) => n !== event.nickname);
            return {
              typingUsers: { ...s.typingUsers, [key]: filtered },
            };
          });
        }, 8000));
        break;
      }

      case 'join': {
        const key = channelKey(event.server_id, event.channel);
        const memberInfo: MemberInfo = {
          nickname: event.nickname,
          avatar_url: event.avatar_url,
          user_id: event.user_id,
          server_avatar_url: event.server_avatar_url,
          role_ids: event.role_ids ?? [],
        };
        set((s) => {
          const current = s.members[key] || [];
          if (current.some((m) => m.nickname === event.nickname)) return s;
          return {
            members: {
              ...s.members,
              [key]: [...current, memberInfo],
            },
            avatars: cacheAvatar(s.avatars, event.nickname, event.avatar_url),
            ...(event.user_id && s.roleProjectionVersions[event.server_id] === undefined ? {
              memberRoles: {
                ...s.memberRoles,
                [event.server_id]: {
                  ...(s.memberRoles[event.server_id] ?? {}),
                  [event.user_id]: event.role_ids ?? [],
                },
              },
            } : {}),
          };
        });
        break;
      }

      case 'part': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          members: {
            ...s.members,
            [key]: (s.members[key] || []).filter(
              (m) => m.nickname !== event.nickname,
            ),
          },
        }));
        break;
      }

      case 'quit': {
        set((s) => {
          const newMembers = { ...s.members };
          for (const ch in newMembers) {
            newMembers[ch] = newMembers[ch].filter((m) => m.nickname !== event.nickname);
          }
          return { members: newMembers };
        });
        break;
      }

      case 'names': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => {
          const newAvatars = { ...s.avatars };
          const serverAvs = { ...(s.serverAvatars[event.server_id] || {}) };
          let serverAvsChanged = false;
          for (const m of event.members) {
            if (m.avatar_url) {
              newAvatars[m.nickname] = m.avatar_url;
            }
            if (m.server_avatar_url && m.user_id) {
              serverAvs[m.user_id] = m.server_avatar_url;
              serverAvsChanged = true;
            }
          }
          return {
            members: { ...s.members, [key]: event.members },
            ...(s.roleProjectionVersions[event.server_id] === undefined ? {
              memberRoles: {
                ...s.memberRoles,
                [event.server_id]: {
                  ...(s.memberRoles[event.server_id] ?? {}),
                  ...Object.fromEntries(event.members.flatMap((member) => member.user_id ? [[member.user_id, member.role_ids ?? []] as const] : [])),
                },
              },
            } : {}),
            avatars: newAvatars,
            ...(serverAvsChanged ? { serverAvatars: { ...s.serverAvatars, [event.server_id]: serverAvs } } : {}),
          };
        });
        break;
      }

      case 'topic_change': {
        set((s) => {
          const serverChannels = s.channels[event.server_id];
          if (!serverChannels) return s;
          return {
            channels: {
              ...s.channels,
              [event.server_id]: serverChannels.map((ch) =>
                ch.name === event.channel ? { ...ch, topic: event.topic } : ch,
              ),
            },
          };
        });
        break;
      }

      case 'channel_list': {
        const previous = get().channels[event.server_id] ?? [];
        const previousConversationIds = previous.map((channel) => channel.conversation_id).sort();
        const nextConversationIds = event.channels.map((channel) => channel.conversation_id).sort();
        const subscriptionsChanged = previousConversationIds.length !== nextConversationIds.length
          || previousConversationIds.some((conversation, index) => conversation !== nextConversationIds[index]);
        const shouldLoadServerMetadata = !hydratedServerMetadata.has(event.server_id);
        const removedConversations = new Set(previous
          .filter((old) => !event.channels.some((current) => current.conversation_id === old.conversation_id))
          .map((channel) => channel.conversation_id));
        for (const [requestId, subscriptions] of useConnectionStore.getState().syncSubscriptions) {
          if (subscriptions.some((conversation) => removedConversations.has(conversation))) {
            useConnectionStore.getState().removeSync(requestId);
          }
        }
        const removedKeys = new Set(previous
          .filter((old) => removedConversations.has(old.conversation_id))
          .map((channel) => channelKey(event.server_id, channel.name)));
        const removedMessageIds = new Set(Object.entries(get().messages)
          .filter(([key]) => removedKeys.has(key))
          .flatMap(([, messages]) => messages.map((message) => message.id)));
        const channels = { ...get().channels, [event.server_id]: event.channels };
        set((state) => ({
          channels,
          messages: Object.fromEntries(Object.entries(state.messages).filter(([key]) => !removedKeys.has(key))),
          members: Object.fromEntries(Object.entries(state.members).filter(([key]) => !removedKeys.has(key))),
          unreadCounts: Object.fromEntries(Object.entries(state.unreadCounts).filter(([key]) => !removedKeys.has(key))),
          hasMore: Object.fromEntries(Object.entries(state.hasMore).filter(([key]) => !removedKeys.has(key))),
          entityVersions: Object.fromEntries(Object.entries(state.entityVersions).filter(([id]) =>
            ![...removedMessageIds].some((messageId) => id === entityVersionKey('message', messageId)))),
        }));
        const ws = get().ws;
        if (shouldLoadServerMetadata) {
          hydratedServerMetadata.add(event.server_id);
          ws?.send({ type: 'list_roles', server_id: event.server_id });
          ws?.send({ type: 'list_categories', server_id: event.server_id });
          ws?.send({ type: 'get_presences', server_id: event.server_id });
          ws?.send({ type: 'get_notification_settings', server_id: event.server_id });
        }
        if (ws && subscriptionsChanged) {
          requestSync(ws, currentSubscriptions(channels, get().directConversations), undefined, get().syncWindowCursors);
        }
        break;
      }

      case 'history': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => {
          const current = s.messages[key] || [];
          const combined = s.durableMode
            ? [...current, ...event.messages.reverse()]
            : [...event.messages.reverse(), ...current];
          // Deduplicate by message ID
          const seen = new Set<string>();
          const deduped = combined.filter(m => {
            if (seen.has(m.id)) return false;
            seen.add(m.id);
            return true;
          });
          const trimmed = deduped.length > MAX_MESSAGES_PER_CHANNEL
            ? deduped.slice(deduped.length - MAX_MESSAGES_PER_CHANNEL)
            : deduped;
          return {
            messages: {
              ...s.messages,
              [key]: trimmed,
            },
            hasMore: { ...s.hasMore, [key]: event.has_more },
          };
        });
        break;
      }

      case 'server_list': {
        const retained = new Set(event.servers.map((server) => server.id));
        const previous = get();
        const removed = new Set(previous.servers.map((server) => server.id).filter((id) => !retained.has(id)));
        const removedChannelIds = new Set(
          [...removed].flatMap((serverId) => (previous.channels[serverId] ?? []).map((channel) => channel.id)),
        );
        useComposerStore.getState().removeChannelKeys(removed);
        set((state) => ({
          servers: event.servers,
          channels: withoutServers(state.channels, retained),
          messages: withoutChannelKeys(state.messages, removed),
          members: withoutChannelKeys(state.members, removed),
          hasMore: withoutChannelKeys(state.hasMore, removed),
          typingUsers: withoutChannelKeys(state.typingUsers, removed),
          unreadCounts: withoutChannelKeys(state.unreadCounts, removed),
          readSequences: withoutChannelKeys(state.readSequences, removed),
          pinnedMessages: withoutChannelKeys(state.pinnedMessages, removed),
          threads: withoutChannelKeys(state.threads, removed),
          forumTags: Object.fromEntries(Object.entries(state.forumTags).filter(([channelId]) => !removedChannelIds.has(channelId))),
          bookmarks: state.bookmarks.filter((bookmark) => !removedChannelIds.has(bookmark.channel_id)),
          searchResults: state.searchResults?.filter((result) => !removedChannelIds.has(result.channel_id)) ?? null,
          customEmoji: withoutServers(state.customEmoji, retained),
          roles: withoutServers(state.roles, retained),
          categories: withoutServers(state.categories, retained),
          presences: withoutServers(state.presences, retained),
          notificationSettings: withoutServers(state.notificationSettings, retained),
          memberRoles: withoutServers(state.memberRoles, retained),
          channelPermissionOverrides: Object.fromEntries(
            Object.entries(state.channelPermissionOverrides)
              .filter(([channelId]) => !removedChannelIds.has(channelId)),
          ),
          auditLog: withoutServers(state.auditLog, retained),
          bans: withoutServers(state.bans, retained),
          automodRules: withoutServers(state.automodRules, retained),
          invites: withoutServers(state.invites, retained),
          serverEvents: withoutServers(state.serverEvents, retained),
          eventRsvps: {},
          communitySettings: withoutServers(state.communitySettings, retained),
          discoverableServers: state.discoverableServers.filter((server) => !removed.has(server.server_id)),
          channelFollows: {},
          templates: withoutServers(state.templates, retained),
          webhooks: withoutServers(state.webhooks, retained),
          slashCommands: withoutServers(state.slashCommands, retained),
          stickers: withoutServers(state.stickers, retained),
          allUserEmoji: state.allUserEmoji.filter((emoji) => retained.has(emoji.server_id)),
          serverAvatars: withoutServers(state.serverAvatars, retained),
        }));
        const ui = useUiStore.getState();
        if (ui.activeServer && removed.has(ui.activeServer)) {
          const fallback = event.servers[0]?.id ?? null;
          ui.setActiveServer(fallback);
          if (fallback) {
            const firstChannel = get().channels[fallback]?.[0]?.name ?? null;
            if (firstChannel) useUiStore.getState().setActiveChannel(firstChannel);
          }
        }
        if (removed.size) {
          ui.setServerFolders(ui.serverFolders.map((folder) => ({
            ...folder,
            serverIds: folder.serverIds.filter((serverId) => retained.has(serverId)),
          })).filter((folder) => folder.serverIds.length > 0));
        }
        break;
      }

      case 'direct_conversation_list': {
        set({ directConversations: event.conversations });
        const ws = get().ws;
        if (ws) requestSync(ws, currentSubscriptions(get().channels, event.conversations), undefined, get().syncWindowCursors);
        break;
      }

      case 'unread_counts': {
        if (get().durableMode) break;
        set((s) => {
          const newUnread = { ...s.unreadCounts };
          for (const { channel_name, count } of event.counts) {
            const key = channelKey(event.server_id, channel_name);
            newUnread[key] = count;
          }
          return { unreadCounts: newUnread };
        });
        break;
      }

      case 'role_list': {
        set((s) => {
          const currentVersion = s.roleProjectionVersions[event.server_id] ?? -1;
          if (event.version < currentVersion) return s;
          const validRoleIds = new Set(event.roles.map((role) => role.id));
          const currentAssignments = s.memberRoles[event.server_id] ?? {};
          const memberRoles = event.member_roles
            ? Object.fromEntries(event.member_roles.map((member) => [member.user_id, member.role_ids]))
            : Object.fromEntries(Object.entries(currentAssignments).map(([userId, roleIds]) => [
                userId,
                roleIds.filter((roleId) => validRoleIds.has(roleId)),
              ]));
          return {
            roles: { ...s.roles, [event.server_id]: event.roles },
            memberRoles: { ...s.memberRoles, [event.server_id]: memberRoles },
            roleProjectionVersions: { ...s.roleProjectionVersions, [event.server_id]: event.version },
          };
        });
        break;
      }

      case 'role_update': {
        set((s) => {
          const current = s.roles[event.server_id] || [];
          const idx = current.findIndex((r) => r.id === event.role.id);
          const updated = idx >= 0
            ? current.map((r) => (r.id === event.role.id ? event.role : r))
            : [...current, event.role];
          return { roles: { ...s.roles, [event.server_id]: updated } };
        });
        break;
      }

      case 'role_delete': {
        set((s) => ({
          roles: {
            ...s.roles,
            [event.server_id]: (s.roles[event.server_id] || []).filter((r) => r.id !== event.role_id),
          },
          memberRoles: {
            ...s.memberRoles,
            [event.server_id]: Object.fromEntries(Object.entries(s.memberRoles[event.server_id] ?? {}).map(([userId, roleIds]) => [userId, roleIds.filter((roleId) => roleId !== event.role_id)])),
          },
        }));
        break;
      }

      case 'member_role_update': {
        set((state) => {
          const currentVersion = state.roleProjectionVersions[event.server_id] ?? -1;
          if (event.version < currentVersion) return state;
          return {
            memberRoles: {
              ...state.memberRoles,
              [event.server_id]: {
                ...(state.memberRoles[event.server_id] ?? {}),
                [event.user_id]: event.role_ids,
              },
            },
            roleProjectionVersions: {
              ...state.roleProjectionVersions,
              [event.server_id]: event.version,
            },
          };
        });
        break;
      }

      case 'channel_permission_override_list': {
        set((state) => ({
          channelPermissionOverrides: {
            ...state.channelPermissionOverrides,
            [event.channel_id]: event.overrides,
          },
        }));
        break;
      }

      case 'category_list': {
        set((s) => ({
          categories: { ...s.categories, [event.server_id]: event.categories },
        }));
        break;
      }

      case 'category_update': {
        set((s) => {
          const current = s.categories[event.server_id] || [];
          const idx = current.findIndex((c) => c.id === event.category.id);
          const updated = idx >= 0
            ? current.map((c) => (c.id === event.category.id ? event.category : c))
            : [...current, event.category];
          return { categories: { ...s.categories, [event.server_id]: updated } };
        });
        break;
      }

      case 'category_delete': {
        set((s) => ({
          categories: {
            ...s.categories,
            [event.server_id]: (s.categories[event.server_id] || []).filter((c) => c.id !== event.category_id),
          },
        }));
        break;
      }

      case 'channel_reorder': {
        set((s) => {
          const channels = s.channels[event.server_id];
          if (!channels) return s;
          const updated = channels.map((ch) => {
            const pos = event.channels.find((p) => p.id === ch.id);
            if (pos) {
              return { ...ch, position: pos.position, category_id: pos.category_id };
            }
            return ch;
          });
          return { channels: { ...s.channels, [event.server_id]: updated } };
        });
        break;
      }

      case 'presence_update': {
        const { server_id, presence } = event;
        set((s) => ({
          ...(presence.user_id === s.activeAccountId ? { ownPresenceStatus: presence.status } : {}),
          presences: {
            ...s.presences,
            [server_id]: {
              ...s.presences[server_id],
              [presence.user_id]: presence,
            },
          },
        }));
        break;
      }

      case 'presence_list': {
        const { server_id, presences: list } = event;
        const map: Record<string, PresenceInfo> = {};
        for (const p of list) {
          map[p.user_id] = p;
        }
        set((s) => ({
          ...(list.find((presence) => presence.user_id === s.activeAccountId)
            ? { ownPresenceStatus: list.find((presence) => presence.user_id === s.activeAccountId)!.status }
            : {}),
          presences: {
            ...s.presences,
            [server_id]: map,
          },
        }));
        break;
      }

      case 'user_profile': {
        set((s) => ({
          userProfiles: {
            ...s.userProfiles,
            [event.profile.user_id]: event.profile,
          },
        }));
        break;
      }

      case 'own_presence': {
        set({
          ownPresenceStatus: event.effective_status,
          ownRequestedStatus: event.requested_status,
          ownCustomStatus: event.custom_status ?? null,
          ownStatusEmoji: event.status_emoji ?? null,
        });
        break;
      }

      case 'server_nickname_update': {
        set((state) => ({
          members: Object.fromEntries(Object.entries(state.members).map(([key, members]) => [
            key,
            key.startsWith(`${event.server_id}:`)
              ? members.map((member) => member.user_id === event.user_id
                ? {
                    ...member,
                    nickname: event.display_name,
                    server_avatar_url: event.server_avatar_url ?? undefined,
                  }
                : member)
              : members,
          ])),
        }));
        break;
      }

      case 'notification_settings': {
        set((state) => ({
          notificationSettings: { ...state.notificationSettings, [event.server_id]: event.settings },
        }));
        break;
      }

      case 'search_results': {
        const activeRequestId = get().activeSearchRequestId;
        if (activeRequestId !== null
          ? event.request_id !== activeRequestId
          : event.request_id !== undefined && event.request_id !== null) break;
        set((state) => ({
          searchResults: event.results.filter((result) => !state.deletedMessageIds[result.id]),
          searchQuery: event.query,
          searchTotalCount: event.total_count,
          searchOffset: event.offset,
          searchNextContinuation: event.next_continuation ?? null,
          searchContinuationTokens: event.restarted
            ? (event.next_continuation ? { [String(event.results.length)]: event.next_continuation } : {})
            : (event.next_continuation
              ? { ...state.searchContinuationTokens, [String(event.offset + event.results.length)]: event.next_continuation }
              : state.searchContinuationTokens),
          activeSearchRequestId: null,
          searchRestarted: event.restarted,
        }));
        break;
      }

      case 'message_pin': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          pinnedMessages: {
            ...s.pinnedMessages,
            [key]: [...(s.pinnedMessages[key] || []), event.pin],
          },
        }));
        break;
      }

      case 'message_unpin': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          pinnedMessages: {
            ...s.pinnedMessages,
            [key]: (s.pinnedMessages[key] || []).filter((p) => p.message_id !== event.message_id),
          },
        }));
        break;
      }

      case 'pinned_messages': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          pinnedMessages: { ...s.pinnedMessages, [key]: event.pins },
        }));
        break;
      }

      case 'thread_create': {
        const key = channelKey(event.server_id, event.parent_channel);
        set((s) => ({
          threads: {
            ...s.threads,
            [key]: [...(s.threads[key] || []), event.thread],
          },
        }));
        break;
      }

      case 'thread_update': {
        set((s) => {
          const newThreads = { ...s.threads };
          for (const ch in newThreads) {
            const idx = newThreads[ch].findIndex((t) => t.id === event.thread.id);
            if (idx >= 0) {
              newThreads[ch] = newThreads[ch].map((t) =>
                t.id === event.thread.id ? event.thread : t,
              );
              break;
            }
          }
          return { threads: newThreads };
        });
        break;
      }

      case 'thread_list': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          threads: { ...s.threads, [key]: event.threads },
          entityVersions: event.threads.reduce((versions, thread) => ({
            ...versions,
            [entityVersionKey('thread_state', thread.id)]: Math.max(
              versions[entityVersionKey('thread_state', thread.id)] ?? 0,
              thread.state_version ?? 0,
            ),
            [entityVersionKey('thread_tags', thread.id)]: Math.max(
              versions[entityVersionKey('thread_tags', thread.id)] ?? 0,
              thread.tags_version ?? 0,
            ),
          }), s.entityVersions),
        }));
        break;
      }

      case 'forum_tag_list': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          forumTags: { ...s.forumTags, [key]: event.tags },
        }));
        break;
      }

      case 'forum_tag_update': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => {
          const current = s.forumTags[key] || [];
          const idx = current.findIndex((t) => t.id === event.tag.id);
          const updated = idx >= 0
            ? current.map((t) => (t.id === event.tag.id ? event.tag : t))
            : [...current, event.tag];
          return { forumTags: { ...s.forumTags, [key]: updated } };
        });
        break;
      }

      case 'forum_tag_delete': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          forumTags: {
            ...s.forumTags,
            [key]: (s.forumTags[key] || []).filter((t) => t.id !== event.tag_id),
          },
        }));
        break;
      }

      case 'thread_tag_update': {
        set((s) => {
          const versionKey = entityVersionKey('thread_tags', event.thread_id);
          if ((s.entityVersions[versionKey] ?? 0) >= event.version) return {};
          return {
            threads: Object.fromEntries(Object.entries(s.threads).map(([parent, entries]) => [
              parent,
              entries.map((thread) => thread.id === event.thread_id
                ? { ...thread, tag_ids: event.tag_ids, tags_version: event.version }
                : thread),
            ])),
            entityVersions: { ...s.entityVersions, [versionKey]: event.version },
          };
        });
        break;
      }

      case 'bookmark_list': {
        set({ bookmarks: event.bookmarks });
        break;
      }

      case 'bookmark_add': {
        set((s) => ({
          bookmarks: [...s.bookmarks, event.bookmark],
        }));
        break;
      }

      case 'bookmark_remove': {
        set((s) => ({
          bookmarks: s.bookmarks.filter((b) => b.message_id !== event.message_id),
        }));
        break;
      }

      // ── Phase 6: Moderation events ──
      case 'member_kick': {
        const e = event as Extract<ServerEvent, { type: 'member_kick' }>;
        const prefix = e.server_id + ':';
        const newMembers = { ...get().members };
        for (const key of Object.keys(newMembers)) {
          if (key.startsWith(prefix)) {
            newMembers[key] = newMembers[key].filter(m => m.user_id !== e.user_id);
          }
        }
        set({ members: newMembers });
        break;
      }
      case 'member_ban': {
        const e = event as Extract<ServerEvent, { type: 'member_ban' }>;
        const prefix = e.server_id + ':';
        const newMembers = { ...get().members };
        for (const key of Object.keys(newMembers)) {
          if (key.startsWith(prefix)) {
            newMembers[key] = newMembers[key].filter(m => m.user_id !== e.user_id);
          }
        }
        set({ members: newMembers });
        break;
      }
      case 'member_unban':
        // No UI action needed — the ban list will be refreshed if viewing it
        break;
      case 'member_timeout':
        // Could update member UI to show timeout badge — for now just acknowledge
        break;
      case 'slow_mode_update': {
        const e = event as Extract<ServerEvent, { type: 'slow_mode_update' }>;
        const channels = get().channels[e.server_id] ?? [];
        set({
          channels: {
            ...get().channels,
            [e.server_id]: channels.map(ch =>
              ch.name === e.channel ? { ...ch, slowmode_seconds: e.seconds } : ch
            ),
          },
        });
        break;
      }
      case 'nsfw_update': {
        const e = event as Extract<ServerEvent, { type: 'nsfw_update' }>;
        const channels = get().channels[e.server_id] ?? [];
        set({
          channels: {
            ...get().channels,
            [e.server_id]: channels.map(ch =>
              ch.name === e.channel ? { ...ch, is_nsfw: e.is_nsfw } : ch
            ),
          },
        });
        break;
      }
      case 'bulk_message_delete': {
        const e = event as Extract<ServerEvent, { type: 'bulk_message_delete' }>;
        const key = channelKey(e.server_id, e.channel);
        const deleteSet = new Set(e.message_ids);
        set((state) => ({
          messages: {
            ...state.messages,
            [key]: redactDeletedReplyPreviews(
              (state.messages[key] ?? []).filter((message) => !deleteSet.has(message.id)), deleteSet,
            ),
          },
          ...removeDeletedReferences(state, deleteSet),
          deletedMessageIds: {
            ...state.deletedMessageIds,
            ...Object.fromEntries(e.message_ids.map((id) => [id, true as const])),
          },
        }));
        break;
      }
      case 'audit_log_entries': {
        const e = event as Extract<ServerEvent, { type: 'audit_log_entries' }>;
        set({
          auditLog: {
            ...get().auditLog,
            [e.server_id]: e.entries,
          },
        });
        break;
      }
      case 'ban_list': {
        const e = event as Extract<ServerEvent, { type: 'ban_list' }>;
        set({
          bans: {
            ...get().bans,
            [e.server_id]: e.bans,
          },
        });
        break;
      }
      case 'automod_rule_list': {
        const e = event as Extract<ServerEvent, { type: 'automod_rule_list' }>;
        set({
          automodRules: {
            ...get().automodRules,
            [e.server_id]: e.rules,
          },
        });
        break;
      }
      case 'automod_rule_update': {
        const e = event as Extract<ServerEvent, { type: 'automod_rule_update' }>;
        const existing = get().automodRules[e.server_id] ?? [];
        const idx = existing.findIndex(r => r.id === e.rule.id);
        const updated = idx >= 0
          ? existing.map(r => r.id === e.rule.id ? e.rule : r)
          : [...existing, e.rule];
        set({
          automodRules: {
            ...get().automodRules,
            [e.server_id]: updated,
          },
        });
        break;
      }
      case 'automod_rule_delete': {
        const e = event as Extract<ServerEvent, { type: 'automod_rule_delete' }>;
        const existing = get().automodRules[e.server_id] ?? [];
        set({
          automodRules: {
            ...get().automodRules,
            [e.server_id]: existing.filter(r => r.id !== e.rule_id),
          },
        });
        break;
      }

      // ── Phase 7: Community & Discovery events ──
      case 'invite_list':
        set({ invites: { ...get().invites, [event.server_id]: event.invites } });
        break;
      case 'invite_create':
        set({ invites: { ...get().invites, [event.server_id]: [...(get().invites[event.server_id] || []), event.invite] } });
        break;
      case 'invite_delete':
        set({ invites: { ...get().invites, [event.server_id]: (get().invites[event.server_id] || []).filter(i => i.id !== event.invite_id) } });
        break;
      case 'event_list':
        set({ serverEvents: { ...get().serverEvents, [event.server_id]: event.events } });
        break;
      case 'event_update': {
        const existing = get().serverEvents[event.server_id] || [];
        const idx = existing.findIndex(e => e.id === event.event.id);
        const updated = idx >= 0 ? [...existing.slice(0, idx), event.event, ...existing.slice(idx + 1)] : [...existing, event.event];
        set({ serverEvents: { ...get().serverEvents, [event.server_id]: updated } });
        break;
      }
      case 'event_delete':
        set({ serverEvents: { ...get().serverEvents, [event.server_id]: (get().serverEvents[event.server_id] || []).filter(e => e.id !== event.event_id) } });
        break;
      case 'event_rsvp_list':
        set({ eventRsvps: { ...get().eventRsvps, [event.event_id]: event.rsvps } });
        break;
      case 'server_community':
        set({ communitySettings: { ...get().communitySettings, [event.community.server_id]: event.community } });
        break;
      case 'discover_servers':
        set({ discoverableServers: event.servers });
        break;
      case 'channel_follow_list':
        set({ channelFollows: { ...get().channelFollows, [event.channel_id]: event.follows } });
        break;
      case 'channel_follow_create': {
        const existing = get().channelFollows[event.follow.source_channel_id] ?? [];
        set({
          channelFollows: {
            ...get().channelFollows,
            [event.follow.source_channel_id]: [
              ...existing.filter((follow) => follow.id !== event.follow.id),
              event.follow,
            ],
          },
        });
        break;
      }
      case 'channel_follow_delete':
        set({
          channelFollows: Object.fromEntries(
            Object.entries(get().channelFollows).map(([channelId, follows]) => [
              channelId,
              follows.filter((follow) => follow.id !== event.follow_id),
            ]),
          ),
        });
        break;
      case 'template_list':
        set({ templates: { ...get().templates, [event.server_id]: event.templates } });
        break;
      case 'template_update': {
        const existing = get().templates[event.server_id] || [];
        const idx = existing.findIndex(t => t.id === event.template.id);
        const updated = idx >= 0 ? [...existing.slice(0, idx), event.template, ...existing.slice(idx + 1)] : [...existing, event.template];
        set({ templates: { ...get().templates, [event.server_id]: updated } });
        break;
      }
      case 'template_delete':
        set({ templates: { ...get().templates, [event.server_id]: (get().templates[event.server_id] || []).filter(t => t.id !== event.template_id) } });
        break;
      case 'template_instantiated':
        get().ws?.send({ type: 'list_servers' });
        break;

      // ── Phase 8: Integrations & Bots ──
      case 'webhook_list':
        set({ webhooks: { ...get().webhooks, [event.server_id]: event.webhooks } });
        break;
      case 'webhook_update': {
        const existing = get().webhooks[event.server_id] || [];
        const idx = existing.findIndex(w => w.id === event.webhook.id);
        const updated = idx >= 0 ? [...existing.slice(0, idx), event.webhook, ...existing.slice(idx + 1)] : [...existing, event.webhook];
        set({ webhooks: { ...get().webhooks, [event.server_id]: updated } });
        break;
      }
      case 'webhook_delete':
        set({ webhooks: { ...get().webhooks, [event.server_id]: (get().webhooks[event.server_id] || []).filter(w => w.id !== event.webhook_id) } });
        break;
      case 'slash_command_list':
        set({
          slashCommands: {
            ...get().slashCommands,
            [event.server_id]: event.commands.map((command) => ({
              ...command,
              server_id: event.server_id,
              created_at: '',
              options: command.options.map((option) => ({
                ...option,
                required: option.required ?? false,
                choices: option.choices ?? undefined,
              })),
            })),
          },
        });
        break;
      case 'slash_command_update': {
        const existing = get().slashCommands[event.server_id] || [];
        const idx = existing.findIndex(c => c.id === event.command.id);
        const command: SlashCommandInfo = {
          ...event.command,
          server_id: event.server_id,
          created_at: '',
          options: event.command.options.map((option) => ({
            ...option,
            required: option.required ?? false,
            choices: option.choices ?? undefined,
          })),
        };
        const updated = idx >= 0 ? [...existing.slice(0, idx), command, ...existing.slice(idx + 1)] : [...existing, command];
        set({ slashCommands: { ...get().slashCommands, [event.server_id]: updated } });
        break;
      }
      case 'slash_command_delete':
        set({ slashCommands: { ...get().slashCommands, [event.server_id]: (get().slashCommands[event.server_id] || []).filter(c => c.id !== event.command_id) } });
        break;
      case 'interaction_create':
        // Interactions are ephemeral — log for debugging
        console.log('Interaction created:', event.interaction);
        break;
      case 'interaction_response':
        set((state) => {
          const key = channelKey(event.server_id, event.channel);
          const message: HistoryMessage = {
            id: `ephemeral:${event.interaction_id}`,
            from: 'Interaction',
            content: event.response.content ?? '',
            timestamp: new Date().toISOString(),
            rich_embeds: event.response.embeds ?? undefined,
            components: event.response.components ?? undefined,
          };
          const current = state.messages[key] ?? [];
          const index = current.findIndex((entry) => entry.id === message.id);
          const messages = index >= 0
            ? [...current.slice(0, index), message, ...current.slice(index + 1)]
            : [...current, message].slice(-MAX_MESSAGES_PER_CHANNEL);
          return { messages: { ...state.messages, [key]: messages } };
        });
        break;
      case 'interaction_invoked': {
        const pending = usePendingStore.getState().takeInteraction(event.request_id);
        if (pending) {
          if (pending.accountGeneration === get().accountGeneration) pending.resolve();
          else pending.reject(new Error('Account changed before the interaction completed.'));
        }
        break;
      }
      case 'lifecycle_command_succeeded': {
        const pending = usePendingStore.getState().takeLifecycle(event.request_id);
        if (pending) {
          if (pending.accountGeneration === get().accountGeneration && pending.connection === get().ws) pending.resolve();
          else pending.reject(new Error('Account changed before the action completed.'));
        }
        break;
      }
      case 'bot_token_list':
        set({ botTokens: event.tokens });
        break;
      case 'bot_account_list':
        set({ botAccounts: event.bots });
        break;
      case 'bot_credential_created':
        set((state) => ({
          botCredential: { botUserId: event.bot_user_id, token: event.token, credential: event.credential },
          botTokens: state.botAccounts.some((bot) => bot.id === event.bot_user_id)
            ? [event.credential, ...state.botTokens.filter((token) => token.id !== event.credential.id)]
            : state.botTokens,
        }));
        break;
      case 'o_auth2_app_list':
        set({ oauth2Apps: event.apps.map((app) => ({ ...app, redirect_uris: app.redirect_uris.join('\n') })) });
        break;
      case 'o_auth2_app_update': {
        const existing = get().oauth2Apps;
        const idx = existing.findIndex(a => a.id === event.app.id);
        const app: OAuth2AppInfo = { ...event.app, redirect_uris: event.app.redirect_uris.join('\n') };
        const updated = idx >= 0 ? [...existing.slice(0, idx), app, ...existing.slice(idx + 1)] : [...existing, app];
        set({ oauth2Apps: updated });
        break;
      }

      case 'server_avatar_update': {
        const sa = { ...get().serverAvatars };
        if (!sa[event.server_id]) sa[event.server_id] = {};
        if (event.avatar_url) {
          sa[event.server_id] = { ...sa[event.server_id], [event.user_id]: event.avatar_url };
        } else {
          const copy = { ...sa[event.server_id] };
          delete copy[event.user_id];
          sa[event.server_id] = copy;
        }
        set({ serverAvatars: sa });
        break;
      }

      case 'server_limits': {
        set({ maxMessageLength: event.max_message_length, maxFileSizeMb: event.max_file_size_mb });
        break;
      }

      case 'error': {
        console.error(`Server error [${event.code}]: ${event.message}`);
        set({ errorToast: event.message });
        setTimeout(() => set({ errorToast: null }), 5000);
        break;
      }
    }
  },

  sendMessage: (serverId, channel, content, attachments) => {
    const { ws, nickname, operationGeneration } = get();
    if (!ws || !nickname) return false;
    if (!operationGeneration) {
      set({ errorToast: 'Synchronizing messages; try again in a moment.' });
      return false;
    }

    const key = channelKey(serverId, channel);
    const composer = useComposerStore.getState();
    const replyingTo = composer.replies[key] ?? composer.replyingTo;

    // Add message locally (server excludes sender from broadcast)
    // Use the same nonce as local ID so the server ack can update it
    const nonce = crypto.randomUUID();
    const msg: HistoryMessage = {
      id: nonce,
      from: nickname,
      content,
      timestamp: new Date().toISOString(),
      reply_to: replyingTo,
      attachments: attachments || null,
    };
    set((s) => {
      const updated = [...(s.messages[key] || []), msg];
      const trimmed = updated.length > MAX_MESSAGES_PER_CHANNEL
        ? updated.slice(updated.length - MAX_MESSAGES_PER_CHANNEL)
        : updated;
      return {
        messages: {
          ...s.messages,
          [key]: trimmed,
        },
      };
    });
    useComposerStore.getState().setReplyFor(key, null);
    useComposerStore.getState().setReplyingTo(null);

    const command = {
      type: 'send_message',
      operation_generation: operationGeneration,
      request_id: nonce,
      client_message_id: nonce,
      server_id: serverId,
      channel,
      content,
      reply_to: replyingTo?.id,
      attachment_ids: attachments?.map((a) => a.id),
      nonce,
    } satisfies ClientCommand;
    pendingCommandOwners.set(nonce, get().accountGeneration);
    set((state) => ({ pendingCommands: { ...state.pendingCommands, [nonce]: command } }));
    const accepted = ws.send(command);
    if (!accepted) {
      pendingCommandOwners.delete(nonce);
      const activeAccountId = get().activeAccountId;
      if (activeAccountId) {
        const composer = useComposerStore.getState();
        composer.setFailedCompositions([...composer.failedCompositions, {
          id: nonce, accountId: activeAccountId, serverId, channel, key, content,
          attachments: attachments ?? [], replyTo: replyingTo,
          error: 'Message was not sent; reconnecting.',
        }]);
      }
      set((state) => ({
        messages: {
          ...state.messages,
          [key]: (state.messages[key] ?? []).filter((message) => message.id !== nonce),
        },
        pendingCommands: Object.fromEntries(
          Object.entries(state.pendingCommands).filter(([requestId]) => requestId !== nonce),
        ),
        errorToast: 'Message was not sent; reconnecting.',
      }));
    }
    return accepted;
  },

  sendDirectMessage: (conversationId, recipient, content, attachments) => {
    const { ws, nickname, operationGeneration } = get();
    if (!ws || !nickname || !operationGeneration) return false;
    const key = `dm:${conversationId}`;
    const composer = useComposerStore.getState();
    const replyingTo = composer.replies[key] ?? composer.replyingTo;
    const nonce = crypto.randomUUID();
    const msg: HistoryMessage = {
      id: nonce,
      from: nickname,
      content,
      timestamp: new Date().toISOString(),
      reply_to: replyingTo,
      attachments: attachments || null,
    };
    set((state) => ({
      messages: { ...state.messages, [key]: [...(state.messages[key] ?? []), msg].slice(-MAX_MESSAGES_PER_CHANNEL) },
    }));
    useComposerStore.getState().setReplyFor(key, null);
    useComposerStore.getState().setReplyingTo(null);
    const command = {
      type: 'send_direct_message',
      operation_generation: operationGeneration,
      request_id: nonce,
      client_message_id: nonce,
      recipient,
      content,
      reply_to: replyingTo?.id,
      attachment_ids: attachments?.map((attachment) => attachment.id),
      nonce,
    } satisfies ClientCommand;
    pendingCommandOwners.set(nonce, get().accountGeneration);
    set((state) => ({ pendingCommands: { ...state.pendingCommands, [nonce]: command } }));
    if (ws.send(command)) return true;
    pendingCommandOwners.delete(nonce);
    const activeAccountId = get().activeAccountId;
    if (activeAccountId) {
      const composer = useComposerStore.getState();
      composer.setFailedCompositions([...composer.failedCompositions, {
        id: nonce, accountId: activeAccountId, serverId: null, channel: recipient,
        conversationId, recipient, key, content, attachments: attachments ?? [], replyTo: replyingTo,
        error: 'Message was not sent; reconnecting.',
      }]);
    }
    set((state) => ({
      messages: { ...state.messages, [key]: (state.messages[key] ?? []).filter((message) => message.id !== nonce) },
      pendingCommands: Object.fromEntries(Object.entries(state.pendingCommands).filter(([id]) => id !== nonce)),
      errorToast: 'Message was not sent; reconnecting.',
    }));
    return false;
  },

  listDirectConversations: () => {
    get().ws?.send({ type: 'list_direct_conversations' });
  },

  editMessage: (messageId, content) => {
    const { ws, operationGeneration } = get();
    if (!ws || !operationGeneration) return;
    const operationId = crypto.randomUUID();
    get().sendTracked(operationId, { type: 'edit_message', operation_generation: operationGeneration, request_id: operationId, client_message_id: operationId, message_id: messageId, content });
  },

  deleteMessage: (messageId) => {
    const { ws, operationGeneration } = get();
    if (!ws || !operationGeneration) return;
    const operationId = crypto.randomUUID();
    get().sendTracked(operationId, { type: 'delete_message', operation_generation: operationGeneration, request_id: operationId, client_message_id: operationId, message_id: messageId });
  },

  addReaction: (messageId, emoji) => {
    const { ws, operationGeneration } = get();
    if (!ws || !operationGeneration) return;
    const operationId = crypto.randomUUID();
    get().sendTracked(operationId, { type: 'add_reaction', operation_generation: operationGeneration, request_id: operationId, client_message_id: operationId, message_id: messageId, emoji });
  },

  removeReaction: (messageId, emoji) => {
    const { ws, operationGeneration } = get();
    if (!ws || !operationGeneration) return;
    const operationId = crypto.randomUUID();
    get().sendTracked(operationId, { type: 'remove_reaction', operation_generation: operationGeneration, request_id: operationId, client_message_id: operationId, message_id: messageId, emoji });
  },

  sendTyping: (serverId, channel) => {
    get().ws?.send({ type: 'typing', server_id: serverId, channel });
  },

  setReplyingTo: (reply) => {
    const ui = useUiStore.getState();
    const key = ui.activeDirectConversation
      ? `dm:${ui.activeDirectConversation}`
      : ui.activeServer && ui.activeChannel ? channelKey(ui.activeServer, ui.activeChannel) : null;
    if (key) useComposerStore.getState().setReplyFor(key, reply);
    useComposerStore.getState().setReplyingTo(reply);
  },

  markRead: (serverId, channel, messageId) => {
    const key = channelKey(serverId, channel);
    const { ws, operationGeneration } = get();
    if (!ws || !operationGeneration) return;
    const operationId = crypto.randomUUID();
    get().sendTracked(operationId, { type: 'mark_read', operation_generation: operationGeneration, request_id: operationId, client_message_id: operationId, server_id: serverId, channel, message_id: messageId });
    // Optimistically clear unread count
    set((s) => {
      const newUnread = { ...s.unreadCounts };
      delete newUnread[key];
      return { unreadCounts: newUnread };
    });
  },

  markDirectRead: (conversationId, messageId) => {
    const { ws, operationGeneration } = get();
    if (!ws || !operationGeneration) return;
    const operationId = crypto.randomUUID();
    get().sendTracked(operationId, {
      type: 'mark_read', operation_generation: operationGeneration,
      request_id: operationId, client_message_id: operationId,
      conversation_id: conversationId, channel: '', message_id: messageId,
    });
    set((state) => ({
      unreadCounts: Object.fromEntries(Object.entries(state.unreadCounts).filter(([key]) => key !== `dm:${conversationId}`)),
      directConversations: state.directConversations.map((conversation) =>
        conversation.id === conversationId ? { ...conversation, unread_count: 0 } : conversation),
    }));
  },

  getUnreadCounts: (serverId) => {
    get().ws?.send({ type: 'get_unread_counts', server_id: serverId });
  },

  joinChannel: (serverId, channel) => {
    get().ws?.send({ type: 'join_channel', server_id: serverId, channel });
  },

  partChannel: (serverId, channel) => {
    get().ws?.send({ type: 'part_channel', server_id: serverId, channel });
  },

  setTopic: (serverId, channel, topic) => {
    get().ws?.send({ type: 'set_topic', server_id: serverId, channel, topic });
  },

  fetchHistory: (serverId, channel, before) => {
    get().ws?.send({ type: 'fetch_history', server_id: serverId, channel, before, limit: 50 });
  },

  listChannels: (serverId) => {
    get().ws?.send({ type: 'list_channels', server_id: serverId });
  },

  getMembers: (serverId, channel) => {
    get().ws?.send({ type: 'get_members', server_id: serverId, channel });
  },

  listServers: () => {
    get().ws?.send({ type: 'list_servers' });
  },

  createServer: (name, iconUrl) => {
    return get().runLifecycleCommand(
      { type: 'create_server', name, icon_url: iconUrl },
      'server:create',
    );
  },

  joinServer: (serverId) => {
    get().ws?.send({ type: 'join_server', server_id: serverId });
  },

  leaveServer: (serverId) => {
    get().ws?.send({ type: 'leave_server', server_id: serverId });
  },

  createChannel: (serverId, name, categoryId, isPrivate, channelType) => {
    get().ws?.send({ type: 'create_channel', server_id: serverId, name, category_id: categoryId, is_private: isPrivate, channel_type: channelType });
  },

  deleteChannel: (serverId, channel) => {
    get().ws?.send({ type: 'delete_channel', server_id: serverId, channel });
  },

  deleteServer: (serverId) => {
    return get().runLifecycleCommand(
      { type: 'delete_server', server_id: serverId },
      `server:${serverId}:delete`,
    );
  },

  updateServer: (serverId, name, iconUrl) => {
    return get().runLifecycleCommand(
      { type: 'update_server', server_id: serverId, name, icon_url: iconUrl },
      `server:${serverId}:update`,
    );
  },

  loadServerEmoji: (serverId) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    return listServerEmoji(serverId)
      .then((emojis) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        const map: Record<string, { id: string; image_url: string }> = {};
        for (const e of emojis) {
          map[e.name] = { id: e.id, image_url: e.image_url };
        }
        set((s) => ({
          customEmoji: { ...s.customEmoji, [serverId]: map },
        }));
      })
      .catch((err) => {
        console.error('Failed to load emoji for server', serverId, err);
      });
  },

  createEmoji: async (serverId, name, imageUrl) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    await createServerEmoji(serverId, name, imageUrl);
    if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
    get().loadServerEmoji(serverId);
  },

  deleteEmoji: async (serverId, emojiId) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    await deleteServerEmoji(serverId, emojiId);
    if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
    get().loadServerEmoji(serverId);
  },

  listRoles: (serverId) => {
    get().ws?.send({ type: 'list_roles', server_id: serverId });
  },

  createRole: (serverId, name, color, permissions) => {
    get().ws?.send({ type: 'create_role', server_id: serverId, name, color, permissions });
  },

  updateRole: (serverId, roleId, updates) => {
    const role = (get().roles[serverId] || []).find((candidate) => candidate.id === roleId);
    if (!role) return;
    get().ws?.send({
      type: 'update_role',
      server_id: serverId,
      role_id: roleId,
      name: updates.name ?? role.name,
      color: updates.color ?? role.color,
      permissions: updates.permissions ?? role.permissions,
    });
  },

  deleteRole: (serverId, roleId) => {
    get().ws?.send({ type: 'delete_role', server_id: serverId, role_id: roleId });
  },

  assignRole: (serverId, userId, roleId) => {
    get().ws?.send({ type: 'assign_role', server_id: serverId, user_id: userId, role_id: roleId });
  },

  removeRole: (serverId, userId, roleId) => {
    get().ws?.send({ type: 'remove_role', server_id: serverId, user_id: userId, role_id: roleId });
  },

  listChannelPermissionOverrides: (serverId, channelId) => {
    get().ws?.send({ type: 'list_channel_permission_overrides', server_id: serverId, channel_id: channelId });
  },

  setChannelPermissionOverride: (serverId, channelId, targetType, targetId, allowBits, denyBits) => {
    get().ws?.send({
      type: 'set_channel_permission_override',
      server_id: serverId,
      channel_id: channelId,
      target_type: targetType,
      target_id: targetId,
      allow_bits: allowBits,
      deny_bits: denyBits,
    });
  },

  deleteChannelPermissionOverride: (serverId, channelId, targetType, targetId) => {
    get().ws?.send({
      type: 'delete_channel_permission_override',
      server_id: serverId,
      channel_id: channelId,
      target_type: targetType,
      target_id: targetId,
    });
  },

  listCategories: (serverId) => {
    get().ws?.send({ type: 'list_categories', server_id: serverId });
  },

  createCategory: (serverId, name) => {
    get().ws?.send({ type: 'create_category', server_id: serverId, name });
  },

  updateCategory: (serverId, categoryId, updates) => {
    const category = (get().categories[serverId] || []).find((candidate) => candidate.id === categoryId);
    if (!category) return;
    get().ws?.send({ type: 'update_category', server_id: serverId, category_id: categoryId, name: updates.name ?? category.name });
  },

  deleteCategory: (serverId, categoryId) => {
    get().ws?.send({ type: 'delete_category', server_id: serverId, category_id: categoryId });
  },

  reorderChannels: (serverId, channels) => {
    get().ws?.send({ type: 'reorder_channels', server_id: serverId, channels });
  },

  setPresence: (status, customStatus, statusEmoji) => {
    get().ws?.send({ type: 'set_presence', status, custom_status: customStatus, status_emoji: statusEmoji });
  },

  getPresences: (serverId) => {
    get().ws?.send({ type: 'get_presences', server_id: serverId });
  },

  setServerNickname: (serverId, nickname) => {
    get().ws?.send({ type: 'set_server_nickname', server_id: serverId, nickname });
  },

  searchMessages: (serverId, query, channel, limit, offset = 0, continuation) => {
    const requestId = crypto.randomUUID();
    if (offset === 0 && !continuation) {
      set({ searchContinuationTokens: {}, searchNextContinuation: null, searchRestarted: false });
    }
    set({ activeSearchRequestId: requestId });
    get().ws?.send({
      type: 'search_messages', request_id: requestId, server_id: serverId, query, channel,
      limit, offset, continuation,
    });
  },

  clearSearch: () => {
    set({
      searchResults: null, searchQuery: '', searchTotalCount: 0, searchOffset: 0,
      searchNextContinuation: null, searchContinuationTokens: {}, activeSearchRequestId: null,
      searchRestarted: false,
    });
  },

  updateNotificationSettings: (serverId, channelId, level, options) => {
    get().ws?.send({
      type: 'update_notification_settings',
      server_id: serverId,
      channel_id: channelId,
      level,
      suppress_everyone: options?.suppressEveryone,
      suppress_roles: options?.suppressRoles,
      muted: options?.muted,
      mute_until: options?.muteUntil,
    });
  },

  getNotificationSettings: (serverId) => {
    get().ws?.send({ type: 'get_notification_settings', server_id: serverId });
  },

  getUserProfile: (userId) => {
    get().ws?.send({ type: 'get_user_profile', user_id: userId });
  },

  pinMessage: (serverId, channel, messageId) => {
    get().ws?.send({ type: 'pin_message', server_id: serverId, channel, message_id: messageId });
  },

  unpinMessage: (serverId, channel, messageId) => {
    get().ws?.send({ type: 'unpin_message', server_id: serverId, channel, message_id: messageId });
  },

  getPinnedMessages: (serverId, channel) => {
    get().ws?.send({ type: 'get_pinned_messages', server_id: serverId, channel });
  },

  createThread: (serverId, parentChannel, name, messageId, isPrivate) => {
    get().ws?.send({ type: 'create_thread', server_id: serverId, parent_channel: parentChannel, name, message_id: messageId, is_private: isPrivate });
  },

  archiveThread: (serverId, threadId) => {
    get().ws?.send({ type: 'archive_thread', server_id: serverId, thread_id: threadId });
  },

  unarchiveThread: (serverId, threadId) => {
    get().ws?.send({ type: 'unarchive_thread', server_id: serverId, thread_id: threadId });
  },

  listThreads: (serverId, channel) => {
    get().ws?.send({ type: 'list_threads', server_id: serverId, channel });
  },

  createForumTag: (serverId, channel, name, emoji, moderated) => {
    get().ws?.send({ type: 'create_forum_tag', server_id: serverId, channel, name, emoji, moderated });
  },

  updateForumTag: (serverId, channel, tag) => {
    get().ws?.send({
      type: 'update_forum_tag',
      server_id: serverId,
      channel,
      tag_id: tag.id,
      name: tag.name,
      emoji: tag.emoji,
      moderated: tag.moderated,
      position: tag.position,
    });
  },

  deleteForumTag: (serverId, channel, tagId) => {
    get().ws?.send({ type: 'delete_forum_tag', server_id: serverId, channel, tag_id: tagId });
  },

  listForumTags: (serverId, channel) => {
    get().ws?.send({ type: 'list_forum_tags', server_id: serverId, channel });
  },

  setThreadTags: (serverId, threadId, tagIds) => {
    get().ws?.send({ type: 'set_thread_tags', server_id: serverId, thread_id: threadId, tag_ids: tagIds });
  },

  getThreadTags: (serverId, threadId) => {
    get().ws?.send({ type: 'get_thread_tags', server_id: serverId, thread_id: threadId });
  },

  addBookmark: (messageId, note) => {
    get().ws?.send({ type: 'add_bookmark', message_id: messageId, note });
  },

  removeBookmark: (messageId) => {
    get().ws?.send({ type: 'remove_bookmark', message_id: messageId });
  },

  listBookmarks: () => {
    get().ws?.send({ type: 'list_bookmarks' });
  },

  // ── Phase 6: Moderation ──
  kickMember: (serverId: string, userId: string, reason?: string) => {
    return get().runLifecycleCommand(
      { type: 'kick_member', server_id: serverId, user_id: userId, reason },
      `moderation:${serverId}:member:${userId}`,
    );
  },
  banMember: (serverId: string, userId: string, reason?: string, deleteMessageDays?: number) => {
    return get().runLifecycleCommand(
      { type: 'ban_member', server_id: serverId, user_id: userId, reason, delete_message_days: deleteMessageDays },
      `moderation:${serverId}:member:${userId}`,
    );
  },
  unbanMember: (serverId: string, userId: string) => {
    return get().runLifecycleCommand(
      { type: 'unban_member', server_id: serverId, user_id: userId },
      `moderation:${serverId}:member:${userId}`,
    );
  },
  listBans: (serverId: string) => {
    get().ws?.send({ type: 'list_bans', server_id: serverId });
  },
  timeoutMember: (serverId: string, userId: string, timeoutUntil?: string, reason?: string) => {
    return get().runLifecycleCommand(
      { type: 'timeout_member', server_id: serverId, user_id: userId, timeout_until: timeoutUntil, reason },
      `moderation:${serverId}:member:${userId}`,
    );
  },
  setSlowMode: (serverId: string, channel: string, seconds: number) => {
    return get().runLifecycleCommand(
      { type: 'set_slow_mode', server_id: serverId, channel, seconds },
      `moderation:${serverId}:channel:${channel}:slowmode`,
    );
  },
  setNsfw: (serverId: string, channel: string, isNsfw: boolean) => {
    return get().runLifecycleCommand(
      { type: 'set_nsfw', server_id: serverId, channel, is_nsfw: isNsfw },
      `moderation:${serverId}:channel:${channel}:nsfw`,
    );
  },
  bulkDeleteMessages: (serverId: string, channel: string, messageIds: string[]) => {
    return get().runLifecycleCommand(
      { type: 'bulk_delete_messages', server_id: serverId, channel, message_ids: messageIds },
      `moderation:${serverId}:channel:${channel}:bulk-delete`,
    );
  },
  getAuditLog: (serverId: string, actionType?: string, limit?: number, before?: string) => {
    get().ws?.send({ type: 'get_audit_log', server_id: serverId, action_type: actionType, limit, before });
  },
  createAutomodRule: (serverId: string, name: string, ruleType: string, config: string, actionType: string, timeoutSeconds?: number) => {
    return get().runLifecycleCommand(
      { type: 'create_automod_rule', server_id: serverId, name, rule_type: ruleType, config, action_type: actionType, timeout_duration_seconds: timeoutSeconds },
      `moderation:${serverId}:automod:create`,
    );
  },
  updateAutomodRule: (serverId: string, ruleId: string, name: string, enabled: boolean, config: string, actionType: string, timeoutSeconds?: number) => {
    return get().runLifecycleCommand(
      { type: 'update_automod_rule', server_id: serverId, rule_id: ruleId, name, enabled, config, action_type: actionType, timeout_duration_seconds: timeoutSeconds },
      `moderation:${serverId}:automod:${ruleId}`,
    );
  },
  deleteAutomodRule: (serverId: string, ruleId: string) => {
    return get().runLifecycleCommand(
      { type: 'delete_automod_rule', server_id: serverId, rule_id: ruleId },
      `moderation:${serverId}:automod:${ruleId}`,
    );
  },
  listAutomodRules: (serverId: string) => {
    get().ws?.send({ type: 'list_automod_rules', server_id: serverId });
  },

  // ── Phase 7: Community & Discovery ──
  createInvite: (serverId, maxUses, expiresAt, channelId) => {
    return get().runLifecycleCommand(
      { type: 'create_invite', server_id: serverId, max_uses: maxUses, expires_at: expiresAt, channel_id: channelId },
      `community:${serverId}:invite:create`,
    );
  },
  listInvites: (serverId) => {
    get().ws?.send({ type: 'list_invites', server_id: serverId });
  },
  deleteInvite: (serverId, inviteId) => {
    return get().runLifecycleCommand(
      { type: 'delete_invite', server_id: serverId, invite_id: inviteId },
      `community:${serverId}:invite:${inviteId}`,
    );
  },
  useInvite: (code) => {
    return get().runLifecycleCommand(
      { type: 'use_invite', code },
      `community:invite:redeem:${code}`,
    );
  },
  createEvent: (serverId, name, startTime, options) => {
    return get().runLifecycleCommand(
      { type: 'create_event', server_id: serverId, name, start_time: startTime, description: options?.description, channel_id: options?.channelId, end_time: options?.endTime, image_url: options?.imageUrl },
      `community:${serverId}:event:create`,
    );
  },
  listEvents: (serverId) => {
    get().ws?.send({ type: 'list_events', server_id: serverId });
  },
  updateEventStatus: (serverId, eventId, status) => {
    return get().runLifecycleCommand(
      { type: 'update_event_status', server_id: serverId, event_id: eventId, status },
      `community:${serverId}:event:${eventId}`,
    );
  },
  deleteEvent: (serverId, eventId) => {
    return get().runLifecycleCommand(
      { type: 'delete_event', server_id: serverId, event_id: eventId },
      `community:${serverId}:event:${eventId}`,
    );
  },
  setRsvp: (serverId, eventId, status) => {
    return get().runLifecycleCommand(
      { type: 'set_rsvp', server_id: serverId, event_id: eventId, status },
      `community:${serverId}:event:${eventId}:rsvp`,
    );
  },
  removeRsvp: (serverId, eventId) => {
    return get().runLifecycleCommand(
      { type: 'remove_rsvp', server_id: serverId, event_id: eventId },
      `community:${serverId}:event:${eventId}:rsvp`,
    );
  },
  listRsvps: (eventId) => {
    get().ws?.send({ type: 'list_rsvps', event_id: eventId });
  },
  updateCommunitySettings: (serverId, settings) => {
    return get().runLifecycleCommand(
      { type: 'update_community_settings', server_id: serverId, description: settings.description, is_discoverable: settings.isDiscoverable, welcome_message: settings.welcomeMessage, rules_text: settings.rulesText, category: settings.category },
      `community:${serverId}:settings`,
    );
  },
  getCommunitySettings: (serverId) => {
    get().ws?.send({ type: 'get_community_settings', server_id: serverId });
  },
  discoverServers: (category) => {
    get().ws?.send({ type: 'discover_servers', category });
  },
  acceptRules: (serverId) => {
    return get().runLifecycleCommand(
      { type: 'accept_rules', server_id: serverId },
      `community:${serverId}:rules:accept`,
    ).then(() => set((state) => {
      const community = state.communitySettings[serverId];
      return community ? { communitySettings: { ...state.communitySettings, [serverId]: { ...community, rules_accepted: true } } } : {};
    }));
  },
  setAnnouncementChannel: (serverId, channel, isAnnouncement) => {
    return get().runLifecycleCommand(
      { type: 'set_announcement_channel', server_id: serverId, channel, is_announcement: isAnnouncement },
      `community:${serverId}:announcement:${channel}`,
    );
  },
  followChannel: (sourceChannelId, targetChannelId) => {
    return get().runLifecycleCommand(
      { type: 'follow_channel', source_channel_id: sourceChannelId, target_channel_id: targetChannelId },
      `community:follow:${sourceChannelId}:${targetChannelId}`,
    );
  },
  unfollowChannel: (followId) => {
    return get().runLifecycleCommand(
      { type: 'unfollow_channel', follow_id: followId },
      `community:follow:${followId}`,
    );
  },
  listChannelFollows: (channelId) => {
    get().ws?.send({ type: 'list_channel_follows', channel_id: channelId });
  },
  createTemplate: (serverId, name, description) => {
    return get().runLifecycleCommand(
      { type: 'create_template', server_id: serverId, name, description },
      `community:${serverId}:template:create`,
    );
  },
  listTemplates: (serverId) => {
    get().ws?.send({ type: 'list_templates', server_id: serverId });
  },
  deleteTemplate: (serverId, templateId) => {
    return get().runLifecycleCommand(
      { type: 'delete_template', server_id: serverId, template_id: templateId },
      `community:${serverId}:template:${templateId}`,
    );
  },
  instantiateTemplate: (templateId, serverName) => {
    return get().runLifecycleCommand(
      { type: 'instantiate_template', template_id: templateId, server_name: serverName },
      `community:template:${templateId}:instantiate`,
    );
  },
  // ── Phase 8: Integrations & Bots ──
  createWebhook: (serverId, channelId, name, webhookType, url) => {
    get().ws?.send({ type: 'create_webhook', server_id: serverId, channel_id: channelId, name, webhook_type: webhookType, url });
  },
  listWebhooks: (serverId) => {
    get().ws?.send({ type: 'list_webhooks', server_id: serverId });
  },
  updateWebhook: (webhookId, name, avatarUrl) => {
    const webhook = Object.values(get().webhooks).flat().find((candidate) => candidate.id === webhookId);
    if (!webhook) return;
    get().ws?.send({ type: 'update_webhook', webhook_id: webhookId, channel_id: webhook.channel_id, name, avatar_url: avatarUrl });
  },
  deleteWebhook: (webhookId) => {
    get().ws?.send({ type: 'delete_webhook', webhook_id: webhookId });
  },
  createBot: (username) => {
    get().ws?.send({ type: 'create_bot', username });
  },
  listOwnedBots: () => {
    get().ws?.send({ type: 'list_owned_bots' });
  },
  clearBotCredential: () => set({ botCredential: null }),
  createBotToken: (botUserId, name, scopes) => {
    get().ws?.send({
      type: 'create_bot_token',
      bot_user_id: botUserId,
      name: name ?? 'Bot token',
      ...(scopes === undefined ? {} : { scopes }),
    });
  },
  listBotTokens: (botUserId) => {
    get().ws?.send({ type: 'list_bot_tokens', bot_user_id: botUserId });
  },
  deleteBotToken: (tokenId) => {
    get().ws?.send({ type: 'delete_bot_token', token_id: tokenId });
  },
  addBotToServer: (botUserId, serverId) => {
    get().ws?.send({ type: 'add_bot_to_server', bot_user_id: botUserId, server_id: serverId });
  },
  removeBotFromServer: (botUserId, serverId) => {
    get().ws?.send({ type: 'remove_bot_from_server', bot_user_id: botUserId, server_id: serverId });
  },
  registerSlashCommand: (serverId, name, description, optionsJson) => {
    get().ws?.send({ type: 'register_slash_command', server_id: serverId, name, description, options_json: optionsJson });
  },
  listSlashCommands: (serverId) => {
    get().ws?.send({ type: 'list_slash_commands', server_id: serverId });
  },
  deleteSlashCommand: (commandId) => {
    get().ws?.send({ type: 'delete_slash_command', command_id: commandId });
  },
  invokeSlashCommand: (serverId, channelId, commandName, argsJson) => {
    const ws = get().ws;
    if (!ws) return Promise.reject(new Error('Not connected.'));
    const requestId = crypto.randomUUID();
    const result = new Promise<void>((resolve, reject) => {
      usePendingStore.getState().registerInteraction(requestId, { accountGeneration: get().accountGeneration, resolve, reject });
    });
    if (!ws.send({ type: 'invoke_slash_command', request_id: requestId, server_id: serverId, channel: channelId, command_name: commandName, args_json: argsJson })) {
      usePendingStore.getState().takeInteraction(requestId);
      return Promise.reject(new Error('Interaction was not sent; reconnecting.'));
    }
    return result;
  },
  invokeMessageComponent: (messageId, customId, values = []) => {
    const ws = get().ws;
    if (!ws) return Promise.reject(new Error('Not connected.'));
    const requestId = crypto.randomUUID();
    const result = new Promise<void>((resolve, reject) => {
      usePendingStore.getState().registerInteraction(requestId, { accountGeneration: get().accountGeneration, resolve, reject });
    });
    if (!ws.send({ type: 'invoke_message_component', request_id: requestId, message_id: messageId, custom_id: customId, values })) {
      usePendingStore.getState().takeInteraction(requestId);
      return Promise.reject(new Error('Interaction was not sent; reconnecting.'));
    }
    return result;
  },
  createOAuth2App: (name, description, redirectUris, clientType) => {
    get().ws?.send({ type: 'create_o_auth2_app', name, description, redirect_uris: redirectUris.split(',').map((uri) => uri.trim()).filter(Boolean), client_type: clientType });
  },
  listOAuth2Apps: () => {
    get().ws?.send({ type: 'list_o_auth2_apps' });
  },
  deleteOAuth2App: (appId) => {
    get().ws?.send({ type: 'delete_o_auth2_app', app_id: appId });
  },
  // ── Phase 9.5: Premium-for-Free Features ──
  loadServerStickers: (serverId) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    listServerStickers(serverId)
      .then((stickers) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        set((s) => ({ stickers: { ...s.stickers, [serverId]: stickers } }));
      })
      .catch((e) => console.error('Failed to load stickers:', e));
  },
  createSticker: async (serverId, name, imageUrl, description) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    await createServerSticker(serverId, name, imageUrl, description);
    if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
    // Reload stickers for this server
    listServerStickers(serverId)
      .then((stickers) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        set((s) => ({ stickers: { ...s.stickers, [serverId]: stickers } }));
      })
      .catch((e) => console.error('Failed to reload stickers:', e));
  },
  deleteSticker: async (serverId, stickerId) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    await deleteServerSticker(serverId, stickerId);
    if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
    set((s) => ({
      stickers: {
        ...s.stickers,
        [serverId]: (s.stickers[serverId] || []).filter((st) => st.id !== stickerId),
      },
    }));
  },
  loadAllUserEmoji: (targetServerId) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    listAllUserEmoji(targetServerId)
      .then((emoji) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        set({ allUserEmoji: emoji });
      })
      .catch((e) => console.error('Failed to load cross-server emoji:', e));
  },
  setServerAvatar: (serverId, avatarUrl) => {
    get().ws?.send({ type: 'set_server_avatar', server_id: serverId, avatar_url: avatarUrl ?? null });
  },
  setVanityCode: (serverId, vanityCode) => {
    return get().runLifecycleCommand(
      { type: 'set_vanity_code', server_id: serverId, vanity_code: vanityCode ?? null },
      `community:${serverId}:vanity`,
    );
  },
  fetchServerLimits: () => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    getServerLimits()
      .then((limits) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        set({ maxMessageLength: limits.max_message_length, maxFileSizeMb: limits.max_file_size_mb });
      })
      .catch((e) => console.error('Failed to fetch server limits:', e));
  },
  // ── Phase 9: AT Protocol Deep Integration ──
  syncBlueskyProfile: async () => {
    await syncBlueskyProfile();
  },
  fetchBlueskyIdentity: (userId) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    getBlueskyIdentity(userId)
      .then((identity) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        set((s) => ({
          blueskyIdentities: { ...s.blueskyIdentities, [userId]: identity },
        }));
      })
      .catch(() => { /* user may not have Bluesky linked */ });
  },
  shareToBluesky: async (messageId) => {
    return await shareToBluesky(messageId);
  },
  fetchAtprotoSyncSetting: () => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    getAtprotoSyncSetting()
      .then((r) => {
        if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
        set({ atprotoSyncEnabled: r.atproto_sync_enabled });
      })
      .catch(() => { /* not authenticated or no AT Protocol account */ });
  },
  setAtprotoSyncEnabled: async (enabled) => {
    const generation = get().protectedGeneration;
    const accountId = get().activeAccountId;
    const r = await updateAtprotoSyncSetting(enabled);
    if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
    set({ atprotoSyncEnabled: r.atproto_sync_enabled });
  },
  });
});

// Keep the legacy ChatState selectors and imperative setState test/integration
// seam as a compatibility facade while composer state has one canonical owner.
const setChatState = useChatStore.setState;
useConnectionStore.subscribe((state) => {
  if (coordinatedDomainUpdate) return;
  setChatState(Object.fromEntries(CONNECTION_KEYS.map((key) => [key, state[key]])) as Partial<ChatState>);
});
usePendingStore.subscribe(({ pendingCommands }) => {
  if (!coordinatedDomainUpdate) setChatState({ pendingCommands });
});
useEntityStore.subscribe(({ replace: _replace, ...entities }) => {
  void _replace;
  if (!coordinatedDomainUpdate) setChatState(entities);
});
useComposerStore.subscribe(({ drafts, compositionFiles, failedCompositions, replyingTo }) => {
  setChatState({ drafts, compositionFiles, failedCompositions, replyingTo });
});

useChatStore.setState = ((next, replace) => {
  const resolved = typeof next === 'function' ? next(useChatStore.getState()) : next;
  if (resolved && typeof resolved === 'object') {
    updateDomainStores(resolved);
    const composer = useComposerStore.getState();
    if ('drafts' in resolved || 'compositionFiles' in resolved
      || 'failedCompositions' in resolved || 'replyingTo' in resolved) {
      composer.replaceState({
        drafts: resolved.drafts ?? composer.drafts,
        compositionFiles: resolved.compositionFiles ?? composer.compositionFiles,
        failedCompositions: resolved.failedCompositions ?? composer.failedCompositions,
        replyingTo: resolved.replyingTo !== undefined ? resolved.replyingTo : composer.replyingTo,
      });
    }
  }
  if (replace === true) {
    setChatState(resolved as ChatState, true);
  } else {
    setChatState(resolved);
  }
}) as typeof useChatStore.setState;
