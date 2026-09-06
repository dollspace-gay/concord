import { channelKey } from '../../api/types';
import { useComposerStore } from '../composerStore';
import { useUiStore } from '../uiStore';
import type { ChatState, ChatStoreContext } from './types';

export function createReadActions({ set, get }: ChatStoreContext): Pick<ChatState, 'sendTyping' | 'setReplyingTo' | 'markRead' | 'markDirectRead' | 'getUnreadCounts'> {
  return {
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
    }
  };
}
