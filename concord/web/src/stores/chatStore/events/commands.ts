import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import { useComposerStore } from '../../composerStore';
import { usePendingStore } from '../../pendingStore';
import { pendingCommandOwners, recoveredThisConnection, retriedCommands, retryTimers } from '../runtime';
import type { ChatStoreContext } from '../types';

export function handleCommandsEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'command_error' | 'command_committed' }>) {
  switch (event.type) {
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
  }
}
