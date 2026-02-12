import { create } from 'zustand';
import type { AttachmentInfo, AuditLogEntry, AutomodRuleInfo, BanInfo, BlueskyIdentityInfo, BlueskyShareResult, BookmarkInfo, BotTokenInfo, CategoryInfo, ChannelInfo, ChannelPositionInfo, EventInfo, ForumTagInfo, HistoryMessage, InviteInfo, MemberInfo, OAuth2AppInfo, PinnedMessageInfo, PresenceInfo, ReplyInfo, RoleInfo, SearchResultMessage, ServerCommunityInfo, ServerEvent, ServerInfo, SlashCommandInfo, TemplateInfo, ThreadInfo, UserProfileInfo, WebhookInfo } from '../api/types';
import { listServerEmoji, createServerEmoji, deleteServerEmoji, syncBlueskyProfile, getBlueskyIdentity, shareToBluesky, getAtprotoSyncSetting, updateAtprotoSyncSetting } from '../api/client';
import { channelKey } from '../api/types';
import { WebSocketManager } from '../api/websocket';

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
const EMPTY_EMOJI: Record<string, Record<string, string>> = {};
const EMPTY_ROLES: Record<string, RoleInfo[]> = {};
const EMPTY_CATEGORIES: Record<string, CategoryInfo[]> = {};
const EMPTY_PRESENCES: Record<string, Record<string, PresenceInfo>> = {};
const EMPTY_PROFILES: Record<string, UserProfileInfo> = {};
const EMPTY_PINS: Record<string, PinnedMessageInfo[]> = {};
const EMPTY_THREADS: Record<string, ThreadInfo[]> = {};
const EMPTY_FORUM_TAGS: Record<string, ForumTagInfo[]> = {};
const EMPTY_BOOKMARKS: BookmarkInfo[] = [];
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

interface ChatState {
  connected: boolean;
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
  /** server_id -> { emoji_name -> image_url } */
  customEmoji: Record<string, Record<string, string>>;
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
  /** channelKey -> pinned messages */
  pinnedMessages: Record<string, PinnedMessageInfo[]>;
  /** channelKey -> threads */
  threads: Record<string, ThreadInfo[]>;
  /** channelKey -> forum tags */
  forumTags: Record<string, ForumTagInfo[]>;
  /** Personal bookmarks */
  bookmarks: BookmarkInfo[];
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
  /** server_id -> community settings */
  communitySettings: Record<string, ServerCommunityInfo>;
  /** Discoverable servers */
  discoverableServers: ServerCommunityInfo[];
  /** server_id -> templates */
  templates: Record<string, TemplateInfo[]>;
  /** server_id -> webhooks */
  webhooks: Record<string, WebhookInfo[]>;
  /** server_id -> slash commands */
  slashCommands: Record<string, SlashCommandInfo[]>;
  /** Bot tokens (for current user's bots) */
  botTokens: BotTokenInfo[];
  /** OAuth2 apps (for current user) */
  oauth2Apps: OAuth2AppInfo[];
  /** user_id -> Bluesky identity info (cached) */
  blueskyIdentities: Record<string, BlueskyIdentityInfo>;
  /** Whether current user has AT Protocol record sync enabled */
  atprotoSyncEnabled: boolean;
  ws: WebSocketManager | null;

  connect: (nickname: string) => void;
  disconnect: () => void;
  handleEvent: (event: ServerEvent) => void;
  sendMessage: (serverId: string, channel: string, content: string, attachments?: AttachmentInfo[]) => void;
  editMessage: (messageId: string, content: string) => void;
  deleteMessage: (messageId: string) => void;
  addReaction: (messageId: string, emoji: string) => void;
  removeReaction: (messageId: string, emoji: string) => void;
  sendTyping: (serverId: string, channel: string) => void;
  setReplyingTo: (reply: ReplyInfo | null) => void;
  markRead: (serverId: string, channel: string, messageId: string) => void;
  getUnreadCounts: (serverId: string) => void;
  joinChannel: (serverId: string, channel: string) => void;
  partChannel: (serverId: string, channel: string) => void;
  setTopic: (serverId: string, channel: string, topic: string) => void;
  fetchHistory: (serverId: string, channel: string, before?: string) => void;
  listChannels: (serverId: string) => void;
  getMembers: (serverId: string, channel: string) => void;
  listServers: () => void;
  createServer: (name: string, iconUrl?: string) => void;
  joinServer: (serverId: string) => void;
  leaveServer: (serverId: string) => void;
  createChannel: (serverId: string, name: string, categoryId?: string, isPrivate?: boolean) => void;
  deleteChannel: (serverId: string, channel: string) => void;
  deleteServer: (serverId: string) => void;
  updateServer: (serverId: string, name?: string, iconUrl?: string) => void;
  loadServerEmoji: (serverId: string) => void;
  createEmoji: (serverId: string, name: string, imageUrl: string) => Promise<void>;
  deleteEmoji: (serverId: string, emojiId: string) => Promise<void>;
  listRoles: (serverId: string) => void;
  createRole: (serverId: string, name: string, color?: string, permissions?: number) => void;
  updateRole: (serverId: string, roleId: string, updates: { name?: string; color?: string; permissions?: number; position?: number }) => void;
  deleteRole: (serverId: string, roleId: string) => void;
  assignRole: (serverId: string, userId: string, roleId: string) => void;
  removeRole: (serverId: string, userId: string, roleId: string) => void;
  listCategories: (serverId: string) => void;
  createCategory: (serverId: string, name: string) => void;
  updateCategory: (serverId: string, categoryId: string, updates: { name?: string; position?: number }) => void;
  deleteCategory: (serverId: string, categoryId: string) => void;
  reorderChannels: (serverId: string, channels: ChannelPositionInfo[]) => void;
  setPresence: (status: string, customStatus?: string, statusEmoji?: string) => void;
  getPresences: (serverId: string) => void;
  setServerNickname: (serverId: string, nickname?: string) => void;
  searchMessages: (serverId: string, query: string, channel?: string, limit?: number, offset?: number) => void;
  clearSearch: () => void;
  updateNotificationSettings: (serverId: string, channelId: string | undefined, level: string, options?: { suppressEveryone?: boolean; suppressRoles?: boolean; muted?: boolean; muteUntil?: string }) => void;
  getNotificationSettings: (serverId: string) => void;
  getUserProfile: (userId: string) => void;
  pinMessage: (serverId: string, channel: string, messageId: string) => void;
  unpinMessage: (serverId: string, channel: string, messageId: string) => void;
  getPinnedMessages: (serverId: string, channel: string) => void;
  createThread: (serverId: string, parentChannel: string, name: string, messageId: string, isPrivate?: boolean) => void;
  archiveThread: (serverId: string, threadId: string) => void;
  listThreads: (serverId: string, channel: string) => void;
  addBookmark: (messageId: string, note?: string) => void;
  removeBookmark: (messageId: string) => void;
  listBookmarks: () => void;
  // ── Phase 6: Moderation ──
  kickMember: (serverId: string, userId: string, reason?: string) => void;
  banMember: (serverId: string, userId: string, reason?: string, deleteMessageDays?: number) => void;
  unbanMember: (serverId: string, userId: string) => void;
  listBans: (serverId: string) => void;
  timeoutMember: (serverId: string, userId: string, timeoutUntil?: string, reason?: string) => void;
  setSlowMode: (serverId: string, channel: string, seconds: number) => void;
  setNsfw: (serverId: string, channel: string, isNsfw: boolean) => void;
  bulkDeleteMessages: (serverId: string, channel: string, messageIds: string[]) => void;
  getAuditLog: (serverId: string, actionType?: string, limit?: number, before?: string) => void;
  createAutomodRule: (serverId: string, name: string, ruleType: string, config: string, actionType: string, timeoutSeconds?: number) => void;
  updateAutomodRule: (serverId: string, ruleId: string, name: string, enabled: boolean, config: string, actionType: string, timeoutSeconds?: number) => void;
  deleteAutomodRule: (serverId: string, ruleId: string) => void;
  listAutomodRules: (serverId: string) => void;
  // ── Phase 7: Community & Discovery ──
  createInvite: (serverId: string, maxUses?: number, expiresAt?: string, channelId?: string) => void;
  listInvites: (serverId: string) => void;
  deleteInvite: (serverId: string, inviteId: string) => void;
  useInvite: (code: string) => void;
  createEvent: (serverId: string, name: string, startTime: string, options?: { description?: string; channelId?: string; endTime?: string; imageUrl?: string }) => void;
  listEvents: (serverId: string) => void;
  updateEventStatus: (serverId: string, eventId: string, status: string) => void;
  deleteEvent: (serverId: string, eventId: string) => void;
  setRsvp: (serverId: string, eventId: string, status: string) => void;
  removeRsvp: (serverId: string, eventId: string) => void;
  listRsvps: (eventId: string) => void;
  updateCommunitySettings: (serverId: string, settings: { description?: string; isDiscoverable: boolean; welcomeMessage?: string; rulesText?: string; category?: string }) => void;
  getCommunitySettings: (serverId: string) => void;
  discoverServers: (category?: string) => void;
  acceptRules: (serverId: string) => void;
  setAnnouncementChannel: (serverId: string, channel: string, isAnnouncement: boolean) => void;
  followChannel: (sourceChannelId: string, targetChannelId: string) => void;
  unfollowChannel: (followId: string) => void;
  listChannelFollows: (channelId: string) => void;
  createTemplate: (serverId: string, name: string, description?: string) => void;
  listTemplates: (serverId: string) => void;
  deleteTemplate: (serverId: string, templateId: string) => void;
  // ── Phase 8: Integrations & Bots ──
  createWebhook: (serverId: string, channelId: string, name: string, webhookType: string, url?: string) => void;
  listWebhooks: (serverId: string) => void;
  updateWebhook: (webhookId: string, name: string, avatarUrl?: string) => void;
  deleteWebhook: (webhookId: string) => void;
  createBot: (username: string) => void;
  createBotToken: (botUserId: string, name?: string, scopes?: string) => void;
  listBotTokens: (botUserId: string) => void;
  deleteBotToken: (tokenId: string) => void;
  addBotToServer: (botUserId: string, serverId: string) => void;
  removeBotFromServer: (botUserId: string, serverId: string) => void;
  registerSlashCommand: (serverId: string, name: string, description: string, optionsJson?: string) => void;
  listSlashCommands: (serverId: string) => void;
  deleteSlashCommand: (commandId: string) => void;
  invokeSlashCommand: (serverId: string, channelId: string, commandName: string, argsJson?: string) => void;
  createOAuth2App: (name: string, description: string, redirectUris: string, scopes?: string) => void;
  listOAuth2Apps: () => void;
  deleteOAuth2App: (appId: string) => void;
  // ── Phase 9: AT Protocol Deep Integration ──
  syncBlueskyProfile: () => Promise<void>;
  fetchBlueskyIdentity: (userId: string) => void;
  shareToBluesky: (messageId: string) => Promise<BlueskyShareResult>;
  fetchAtprotoSyncSetting: () => void;
  setAtprotoSyncEnabled: (enabled: boolean) => Promise<void>;
}

/** Cache an avatar_url for a nickname if present. */
function cacheAvatar(avatars: Record<string, string>, nickname: string, avatar_url?: string | null): Record<string, string> {
  if (avatar_url && avatars[nickname] !== avatar_url) {
    return { ...avatars, [nickname]: avatar_url };
  }
  return avatars;
}

export const useChatStore = create<ChatState>((set, get) => ({
  connected: false,
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
  pinnedMessages: EMPTY_PINS,
  threads: EMPTY_THREADS,
  forumTags: EMPTY_FORUM_TAGS,
  bookmarks: EMPTY_BOOKMARKS,
  auditLog: {} as Record<string, AuditLogEntry[]>,
  bans: {} as Record<string, BanInfo[]>,
  automodRules: {} as Record<string, AutomodRuleInfo[]>,
  invites: EMPTY_INVITES,
  serverEvents: EMPTY_EVENTS,
  communitySettings: EMPTY_COMMUNITY,
  discoverableServers: EMPTY_DISCOVER,
  templates: EMPTY_TEMPLATES,
  webhooks: EMPTY_WEBHOOKS,
  slashCommands: EMPTY_SLASH_COMMANDS,
  botTokens: EMPTY_BOT_TOKENS,
  oauth2Apps: EMPTY_OAUTH2_APPS,
  blueskyIdentities: EMPTY_BLUESKY_IDENTITIES,
  atprotoSyncEnabled: false,
  ws: null,

  connect: (nickname: string) => {
    if (get().ws) {
      return;
    }

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${protocol}//${window.location.host}/ws?nickname=${encodeURIComponent(nickname)}`;

    const ws = new WebSocketManager(
      url,
      (event) => {
        get().handleEvent(event);
      },
      (connected) => {
        set({ connected });
        if (connected) {
          ws.send({ type: 'list_servers' });
        }
      },
    );

    set({ ws, nickname });
    ws.connect();
  },

  disconnect: () => {
    get().ws?.disconnect();
    set({
      ws: null,
      connected: false,
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
      pinnedMessages: EMPTY_PINS,
      threads: EMPTY_THREADS,
      forumTags: EMPTY_FORUM_TAGS,
      bookmarks: EMPTY_BOOKMARKS,
      auditLog: {} as Record<string, AuditLogEntry[]>,
      bans: {} as Record<string, BanInfo[]>,
      automodRules: {} as Record<string, AutomodRuleInfo[]>,
      invites: EMPTY_INVITES,
      serverEvents: EMPTY_EVENTS,
      communitySettings: EMPTY_COMMUNITY,
      discoverableServers: EMPTY_DISCOVER,
      templates: EMPTY_TEMPLATES,
      webhooks: EMPTY_WEBHOOKS,
      slashCommands: EMPTY_SLASH_COMMANDS,
      botTokens: EMPTY_BOT_TOKENS,
      oauth2Apps: EMPTY_OAUTH2_APPS,
      blueskyIdentities: EMPTY_BLUESKY_IDENTITIES,
      atprotoSyncEnabled: false,
    });
  },

  handleEvent: (event: ServerEvent) => {
    switch (event.type) {
      case 'message': {
        const sid = event.server_id || 'default';
        const key = channelKey(sid, event.target);
        const msg: HistoryMessage = {
          id: event.id,
          from: event.from,
          content: event.content,
          timestamp: event.timestamp,
          reply_to: event.reply_to,
          attachments: event.attachments,
        };
        set((s) => {
          // Increment unread count for messages from others
          const newUnread = { ...s.unreadCounts };
          if (event.from !== s.nickname) {
            newUnread[key] = (newUnread[key] || 0) + 1;
          }
          return {
            messages: {
              ...s.messages,
              [key]: [...(s.messages[key] || []), msg],
            },
            avatars: cacheAvatar(s.avatars, event.from, event.avatar_url),
            unreadCounts: newUnread,
          };
        });
        break;
      }

      case 'message_edit': {
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
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: (s.messages[key] || []).filter((m) => m.id !== event.id),
          },
        }));
        break;
      }

      case 'message_ack': {
        // Server sends back the real message ID + nonce — update the optimistic local message
        const ackKey = channelKey(event.server_id, event.channel);
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
        // Auto-expire after 8 seconds
        setTimeout(() => {
          set((s) => {
            const current = s.typingUsers[key] || [];
            const filtered = current.filter((n) => n !== event.nickname);
            return {
              typingUsers: { ...s.typingUsers, [key]: filtered },
            };
          });
        }, 8000);
        break;
      }

      case 'join': {
        const key = channelKey(event.server_id, event.channel);
        const memberInfo: MemberInfo = { nickname: event.nickname, avatar_url: event.avatar_url };
        set((s) => {
          const current = s.members[key] || [];
          if (current.some((m) => m.nickname === event.nickname)) return s;
          return {
            members: {
              ...s.members,
              [key]: [...current, memberInfo],
            },
            avatars: cacheAvatar(s.avatars, event.nickname, event.avatar_url),
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
          let newAvatars = { ...s.avatars };
          for (const m of event.members) {
            if (m.avatar_url) {
              newAvatars[m.nickname] = m.avatar_url;
            }
          }
          return {
            members: { ...s.members, [key]: event.members },
            avatars: newAvatars,
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
        set((s) => ({
          channels: { ...s.channels, [event.server_id]: event.channels },
        }));
        // Also fetch roles and categories for this server
        const ws = get().ws;
        ws?.send({ type: 'list_roles', server_id: event.server_id });
        ws?.send({ type: 'list_categories', server_id: event.server_id });
        // Also fetch presences for this server
        ws?.send({ type: 'get_presences', server_id: event.server_id });
        break;
      }

      case 'history': {
        const key = channelKey(event.server_id, event.channel);
        set((s) => ({
          messages: {
            ...s.messages,
            [key]: [
              ...event.messages.reverse(),
              ...(s.messages[key] || []),
            ],
          },
          hasMore: { ...s.hasMore, [key]: event.has_more },
        }));
        break;
      }

      case 'server_list': {
        set({ servers: event.servers });
        break;
      }

      case 'unread_counts': {
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
        set((s) => ({
          roles: { ...s.roles, [event.server_id]: event.roles },
        }));
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
        }));
        break;
      }

      case 'member_role_update': {
        // For now, log it. Full member-role tracking will be used by MemberList.
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

      case 'server_nickname_update': {
        // Could update member list nickname if needed
        break;
      }

      case 'notification_settings': {
        // Store notification settings in a temporary location if needed
        break;
      }

      case 'search_results': {
        set({
          searchResults: event.results,
          searchQuery: event.query,
          searchTotalCount: event.total_count,
        });
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
        const msgs = get().messages[key] ?? [];
        const deleteSet = new Set(e.message_ids);
        set({
          messages: {
            ...get().messages,
            [key]: msgs.filter(m => !deleteSet.has(m.id)),
          },
        });
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
        // RSVP list — log for now, will be wired to UI later
        console.log('RSVP list for event', event.event_id, event.rsvps);
        break;
      case 'server_community':
        set({ communitySettings: { ...get().communitySettings, [event.community.server_id]: event.community } });
        break;
      case 'discover_servers':
        set({ discoverableServers: event.servers });
        break;
      case 'channel_follow_list':
        // Channel follows — log for now, secondary feature
        console.log('Channel follows for', event.channel_id, event.follows);
        break;
      case 'channel_follow_create':
        console.log('Channel follow created', event.follow);
        break;
      case 'channel_follow_delete':
        console.log('Channel follow deleted', event.follow_id);
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
        set({ slashCommands: { ...get().slashCommands, [event.server_id]: event.commands } });
        break;
      case 'slash_command_update': {
        const existing = get().slashCommands[event.server_id] || [];
        const idx = existing.findIndex(c => c.id === event.command.id);
        const updated = idx >= 0 ? [...existing.slice(0, idx), event.command, ...existing.slice(idx + 1)] : [...existing, event.command];
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
        // Interaction responses are handled as messages by the server
        console.log('Interaction response:', event.interaction_id);
        break;
      case 'bot_token_list':
        set({ botTokens: event.tokens });
        break;
      case 'oauth2_app_list':
        set({ oauth2Apps: event.apps });
        break;
      case 'oauth2_app_update': {
        const existing = get().oauth2Apps;
        const idx = existing.findIndex(a => a.id === event.app.id);
        const updated = idx >= 0 ? [...existing.slice(0, idx), event.app, ...existing.slice(idx + 1)] : [...existing, event.app];
        set({ oauth2Apps: updated });
        break;
      }

      case 'error': {
        console.error(`Server error [${event.code}]: ${event.message}`);
        break;
      }
    }
  },

  sendMessage: (serverId, channel, content, attachments) => {
    const { ws, nickname, replyingTo } = get();
    if (!ws || !nickname) return;

    const key = channelKey(serverId, channel);

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
    set((s) => ({
      messages: {
        ...s.messages,
        [key]: [...(s.messages[key] || []), msg],
      },
      replyingTo: null,
    }));

    ws.send({
      type: 'send_message',
      server_id: serverId,
      channel,
      content,
      reply_to: replyingTo?.id,
      attachment_ids: attachments?.map((a) => a.id),
      nonce,
    });
  },

  editMessage: (messageId, content) => {
    get().ws?.send({ type: 'edit_message', message_id: messageId, content });
  },

  deleteMessage: (messageId) => {
    get().ws?.send({ type: 'delete_message', message_id: messageId });
  },

  addReaction: (messageId, emoji) => {
    get().ws?.send({ type: 'add_reaction', message_id: messageId, emoji });
  },

  removeReaction: (messageId, emoji) => {
    get().ws?.send({ type: 'remove_reaction', message_id: messageId, emoji });
  },

  sendTyping: (serverId, channel) => {
    get().ws?.send({ type: 'typing', server_id: serverId, channel });
  },

  setReplyingTo: (reply) => {
    set({ replyingTo: reply });
  },

  markRead: (serverId, channel, messageId) => {
    const key = channelKey(serverId, channel);
    get().ws?.send({ type: 'mark_read', server_id: serverId, channel, message_id: messageId });
    // Optimistically clear unread count
    set((s) => {
      const newUnread = { ...s.unreadCounts };
      delete newUnread[key];
      return { unreadCounts: newUnread };
    });
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
    get().ws?.send({ type: 'create_server', name, icon_url: iconUrl });
  },

  joinServer: (serverId) => {
    get().ws?.send({ type: 'join_server', server_id: serverId });
  },

  leaveServer: (serverId) => {
    get().ws?.send({ type: 'leave_server', server_id: serverId });
  },

  createChannel: (serverId, name, categoryId, isPrivate) => {
    get().ws?.send({ type: 'create_channel', server_id: serverId, name, category_id: categoryId, is_private: isPrivate });
  },

  deleteChannel: (serverId, channel) => {
    get().ws?.send({ type: 'delete_channel', server_id: serverId, channel });
  },

  deleteServer: (serverId) => {
    get().ws?.send({ type: 'delete_server', server_id: serverId });
  },

  updateServer: (serverId, name, iconUrl) => {
    get().ws?.send({ type: 'update_server', server_id: serverId, name, icon_url: iconUrl });
  },

  loadServerEmoji: (serverId) => {
    listServerEmoji(serverId)
      .then((emojis) => {
        const map: Record<string, string> = {};
        for (const e of emojis) {
          map[e.name] = e.image_url;
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
    await createServerEmoji(serverId, name, imageUrl);
    get().loadServerEmoji(serverId);
  },

  deleteEmoji: async (serverId, emojiId) => {
    await deleteServerEmoji(serverId, emojiId);
    get().loadServerEmoji(serverId);
  },

  listRoles: (serverId) => {
    get().ws?.send({ type: 'list_roles', server_id: serverId });
  },

  createRole: (serverId, name, color, permissions) => {
    get().ws?.send({ type: 'create_role', server_id: serverId, name, color, permissions });
  },

  updateRole: (serverId, roleId, updates) => {
    get().ws?.send({ type: 'update_role', server_id: serverId, role_id: roleId, ...updates });
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

  listCategories: (serverId) => {
    get().ws?.send({ type: 'list_categories', server_id: serverId });
  },

  createCategory: (serverId, name) => {
    get().ws?.send({ type: 'create_category', server_id: serverId, name });
  },

  updateCategory: (serverId, categoryId, updates) => {
    get().ws?.send({ type: 'update_category', server_id: serverId, category_id: categoryId, ...updates });
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

  searchMessages: (serverId, query, channel, limit, offset) => {
    get().ws?.send({ type: 'search_messages', server_id: serverId, query, channel, limit, offset });
  },

  clearSearch: () => {
    set({ searchResults: null, searchQuery: '', searchTotalCount: 0 });
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

  listThreads: (serverId, channel) => {
    get().ws?.send({ type: 'list_threads', server_id: serverId, channel });
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
    get().ws?.send({ type: 'kick_member', server_id: serverId, user_id: userId, reason });
  },
  banMember: (serverId: string, userId: string, reason?: string, deleteMessageDays?: number) => {
    get().ws?.send({ type: 'ban_member', server_id: serverId, user_id: userId, reason, delete_message_days: deleteMessageDays });
  },
  unbanMember: (serverId: string, userId: string) => {
    get().ws?.send({ type: 'unban_member', server_id: serverId, user_id: userId });
  },
  listBans: (serverId: string) => {
    get().ws?.send({ type: 'list_bans', server_id: serverId });
  },
  timeoutMember: (serverId: string, userId: string, timeoutUntil?: string, reason?: string) => {
    get().ws?.send({ type: 'timeout_member', server_id: serverId, user_id: userId, timeout_until: timeoutUntil, reason });
  },
  setSlowMode: (serverId: string, channel: string, seconds: number) => {
    get().ws?.send({ type: 'set_slow_mode', server_id: serverId, channel, seconds });
  },
  setNsfw: (serverId: string, channel: string, isNsfw: boolean) => {
    get().ws?.send({ type: 'set_nsfw', server_id: serverId, channel, is_nsfw: isNsfw });
  },
  bulkDeleteMessages: (serverId: string, channel: string, messageIds: string[]) => {
    get().ws?.send({ type: 'bulk_delete_messages', server_id: serverId, channel, message_ids: messageIds });
  },
  getAuditLog: (serverId: string, actionType?: string, limit?: number, before?: string) => {
    get().ws?.send({ type: 'get_audit_log', server_id: serverId, action_type: actionType, limit, before });
  },
  createAutomodRule: (serverId: string, name: string, ruleType: string, config: string, actionType: string, timeoutSeconds?: number) => {
    get().ws?.send({ type: 'create_automod_rule', server_id: serverId, name, rule_type: ruleType, config, action_type: actionType, timeout_duration_seconds: timeoutSeconds });
  },
  updateAutomodRule: (serverId: string, ruleId: string, name: string, enabled: boolean, config: string, actionType: string, timeoutSeconds?: number) => {
    get().ws?.send({ type: 'update_automod_rule', server_id: serverId, rule_id: ruleId, name, enabled, config, action_type: actionType, timeout_duration_seconds: timeoutSeconds });
  },
  deleteAutomodRule: (serverId: string, ruleId: string) => {
    get().ws?.send({ type: 'delete_automod_rule', server_id: serverId, rule_id: ruleId });
  },
  listAutomodRules: (serverId: string) => {
    get().ws?.send({ type: 'list_automod_rules', server_id: serverId });
  },

  // ── Phase 7: Community & Discovery ──
  createInvite: (serverId, maxUses, expiresAt, channelId) => {
    get().ws?.send({ type: 'create_invite', server_id: serverId, max_uses: maxUses, expires_at: expiresAt, channel_id: channelId });
  },
  listInvites: (serverId) => {
    get().ws?.send({ type: 'list_invites', server_id: serverId });
  },
  deleteInvite: (serverId, inviteId) => {
    get().ws?.send({ type: 'delete_invite', server_id: serverId, invite_id: inviteId });
  },
  useInvite: (code) => {
    get().ws?.send({ type: 'use_invite', code });
  },
  createEvent: (serverId, name, startTime, options) => {
    get().ws?.send({ type: 'create_event', server_id: serverId, name, start_time: startTime, description: options?.description, channel_id: options?.channelId, end_time: options?.endTime, image_url: options?.imageUrl });
  },
  listEvents: (serverId) => {
    get().ws?.send({ type: 'list_events', server_id: serverId });
  },
  updateEventStatus: (serverId, eventId, status) => {
    get().ws?.send({ type: 'update_event_status', server_id: serverId, event_id: eventId, status });
  },
  deleteEvent: (serverId, eventId) => {
    get().ws?.send({ type: 'delete_event', server_id: serverId, event_id: eventId });
  },
  setRsvp: (serverId, eventId, status) => {
    get().ws?.send({ type: 'set_rsvp', server_id: serverId, event_id: eventId, status });
  },
  removeRsvp: (serverId, eventId) => {
    get().ws?.send({ type: 'remove_rsvp', server_id: serverId, event_id: eventId });
  },
  listRsvps: (eventId) => {
    get().ws?.send({ type: 'list_rsvps', event_id: eventId });
  },
  updateCommunitySettings: (serverId, settings) => {
    get().ws?.send({ type: 'update_community_settings', server_id: serverId, description: settings.description, is_discoverable: settings.isDiscoverable, welcome_message: settings.welcomeMessage, rules_text: settings.rulesText, category: settings.category });
  },
  getCommunitySettings: (serverId) => {
    get().ws?.send({ type: 'get_community_settings', server_id: serverId });
  },
  discoverServers: (category) => {
    get().ws?.send({ type: 'discover_servers', category });
  },
  acceptRules: (serverId) => {
    get().ws?.send({ type: 'accept_rules', server_id: serverId });
  },
  setAnnouncementChannel: (serverId, channel, isAnnouncement) => {
    get().ws?.send({ type: 'set_announcement_channel', server_id: serverId, channel, is_announcement: isAnnouncement });
  },
  followChannel: (sourceChannelId, targetChannelId) => {
    get().ws?.send({ type: 'follow_channel', source_channel_id: sourceChannelId, target_channel_id: targetChannelId });
  },
  unfollowChannel: (followId) => {
    get().ws?.send({ type: 'unfollow_channel', follow_id: followId });
  },
  listChannelFollows: (channelId) => {
    get().ws?.send({ type: 'list_channel_follows', channel_id: channelId });
  },
  createTemplate: (serverId, name, description) => {
    get().ws?.send({ type: 'create_template', server_id: serverId, name, description });
  },
  listTemplates: (serverId) => {
    get().ws?.send({ type: 'list_templates', server_id: serverId });
  },
  deleteTemplate: (serverId, templateId) => {
    get().ws?.send({ type: 'delete_template', server_id: serverId, template_id: templateId });
  },
  // ── Phase 8: Integrations & Bots ──
  createWebhook: (serverId, channelId, name, webhookType, url) => {
    get().ws?.send({ type: 'create_webhook', server_id: serverId, channel_id: channelId, name, webhook_type: webhookType, url });
  },
  listWebhooks: (serverId) => {
    get().ws?.send({ type: 'list_webhooks', server_id: serverId });
  },
  updateWebhook: (webhookId, name, avatarUrl) => {
    get().ws?.send({ type: 'update_webhook', webhook_id: webhookId, name, avatar_url: avatarUrl });
  },
  deleteWebhook: (webhookId) => {
    get().ws?.send({ type: 'delete_webhook', webhook_id: webhookId });
  },
  createBot: (username) => {
    get().ws?.send({ type: 'create_bot', username });
  },
  createBotToken: (botUserId, name, scopes) => {
    get().ws?.send({ type: 'create_bot_token', bot_user_id: botUserId, name, scopes });
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
    get().ws?.send({ type: 'invoke_slash_command', server_id: serverId, channel_id: channelId, command_name: commandName, args_json: argsJson });
  },
  createOAuth2App: (name, description, redirectUris, scopes) => {
    get().ws?.send({ type: 'create_oauth2_app', name, description, redirect_uris: redirectUris, scopes });
  },
  listOAuth2Apps: () => {
    get().ws?.send({ type: 'list_oauth2_apps' });
  },
  deleteOAuth2App: (appId) => {
    get().ws?.send({ type: 'delete_oauth2_app', app_id: appId });
  },
  // ── Phase 9: AT Protocol Deep Integration ──
  syncBlueskyProfile: async () => {
    await syncBlueskyProfile();
  },
  fetchBlueskyIdentity: (userId) => {
    getBlueskyIdentity(userId)
      .then((identity) => {
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
    getAtprotoSyncSetting()
      .then((r) => set({ atprotoSyncEnabled: r.atproto_sync_enabled }))
      .catch(() => { /* not authenticated or no AT Protocol account */ });
  },
  setAtprotoSyncEnabled: async (enabled) => {
    const r = await updateAtprotoSyncSetting(enabled);
    set({ atprotoSyncEnabled: r.atproto_sync_enabled });
  },
}));
