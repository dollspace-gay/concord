import { WebSocketManager } from '../../api/websocket';
import { useComposerStore } from '../composerStore';
import { useConnectionStore } from '../connectionStore';
import { EMPTY_TYPING } from './defaults';
import { protectedStateReset } from './reset';
import { draftsByAccount, hydratedServerMetadata, pendingCommandOwners, recoveredThisConnection, retriedCommands, retryTimers } from './runtime';
import { rejectPendingInteractions, requestSync } from './synchronization';
import type { ChatState, ChatStoreContext } from './types';

export function createConnectionActions({ set, get }: ChatStoreContext): Pick<ChatState, 'connect' | 'disconnect'> {
  return {
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
    }
  };
}
