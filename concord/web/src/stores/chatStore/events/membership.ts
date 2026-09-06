import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { MemberInfo } from '../../../api/types';
import { channelKey } from '../../../api/types';
import { useConnectionStore } from '../../connectionStore';
import { cacheAvatar, entityVersionKey } from '../identity';
import { hydratedServerMetadata, typingTimeouts } from '../runtime';
import { currentSubscriptions, requestSync } from '../synchronization';
import type { ChatStoreContext } from '../types';

export function handleMembershipEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'typing_start' | 'join' | 'part' | 'quit' | 'names' | 'topic_change' | 'channel_list' }>) {
  switch (event.type) {
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
  }
}
