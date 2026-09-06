import type { ClientMessage as ClientCommand } from '../../api/generated/contract';
import type { HistoryMessage } from '../../api/types';
import { channelKey } from '../../api/types';
import { useComposerStore } from '../composerStore';
import { MAX_MESSAGES_PER_CHANNEL } from './defaults';
import { pendingCommandOwners } from './runtime';
import type { ChatState, ChatStoreContext } from './types';

export function createMessageActions({ set, get }: ChatStoreContext): Pick<ChatState, 'sendMessage' | 'sendDirectMessage' | 'listDirectConversations' | 'editMessage' | 'deleteMessage' | 'addReaction' | 'removeReaction'> {
  return {
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
    }
  };
}
