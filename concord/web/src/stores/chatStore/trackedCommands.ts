import { LifecycleOutcomeUncertainError, usePendingStore } from '../pendingStore';
import { LIFECYCLE_RESULT_TIMEOUT_MS } from './defaults';
import { pendingCommandOwners } from './runtime';
import type { ChatState, ChatStoreContext } from './types';

export function createTrackedCommands({ set, get }: ChatStoreContext): Pick<ChatState, 'sendTracked' | 'runLifecycleCommand'> {
  return {
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
    }
  };
}
