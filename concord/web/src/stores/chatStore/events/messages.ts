import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { HistoryMessage } from '../../../api/types';
import { channelKey } from '../../../api/types';
import { MAX_MESSAGES_PER_CHANNEL } from '../defaults';
import { cacheAvatar, entityVersionKey } from '../identity';
import { maybeNotifyMessage } from '../notifications';
import { redactDeletedReplyPreviews, removeDeletedReferences } from '../references';
import type { ChatStoreContext } from '../types';

export function handleMessagesEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'message' | 'message_edit' | 'message_delete' | 'message_ack' | 'message_embed' | 'reaction_add' | 'reaction_remove' }>) {
  switch (event.type) {
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
      maybeNotifyMessage(get, get(), {
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
  }
}
