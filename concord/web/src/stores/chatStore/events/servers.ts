import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import { useComposerStore } from '../../composerStore';
import { useUiStore } from '../../uiStore';
import { withoutChannelKeys, withoutServers } from '../references';
import { currentSubscriptions, requestSync } from '../synchronization';
import type { ChatStoreContext } from '../types';

export function handleServersEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'server_list' | 'direct_conversation_list' | 'unread_counts' }>) {
  switch (event.type) {
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
  }
}
