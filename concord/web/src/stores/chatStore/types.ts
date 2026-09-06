import type { StoreApi } from 'zustand';
import type { ChannelPermissionOverrideInfo, ClientMessage as ClientCommand, DirectConversationInfo, ChatEvent as ServerEvent } from '../../api/generated/contract';
import type { AttachmentInfo, AuditLogEntry, AutomodRuleInfo, BanInfo, BlueskyIdentityInfo, BlueskyShareResult, BookmarkInfo, BotAccountInfo, BotTokenInfo, CategoryInfo, ChannelFollowInfo, ChannelInfo, ChannelPositionInfo, EventInfo, ForumTagInfo, HistoryMessage, InviteInfo, MemberInfo, NotificationSettingInfo, OAuth2AppInfo, PinnedMessageInfo, PresenceInfo, ReplyInfo, RoleInfo, RsvpInfo, SearchResultMessage, ServerCommunityInfo, ServerInfo, SlashCommandInfo, StickerInfo, TemplateInfo, ThreadInfo, UserProfileInfo, WebhookInfo } from '../../api/types';
import { WebSocketManager } from '../../api/websocket';
import { type FailedComposition } from '../composerStore';

export interface ChatState {
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

export interface ChatStoreContext {
  set: StoreApi<ChatState>['setState'];
  get: () => ChatState;
}
