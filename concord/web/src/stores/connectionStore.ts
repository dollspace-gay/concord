import { create } from 'zustand';
import type { WebSocketManager } from '../api/websocket';

export interface ConnectionState {
  connected: boolean;
  ws: WebSocketManager | null;
  nickname: string | null;
  activeAccountId: string | null;
  accountGeneration: number;
  protectedGeneration: number;
  operationGeneration: string | null;
  syncCursor: string | null;
  syncWindowCursors: Record<string, string>;
  durableMode: boolean;
  ownPresenceStatus: string | null;
  ownRequestedStatus: string | null;
  ownCustomStatus: string | null;
  ownStatusEmoji: string | null;
  syncSubscriptions: Map<string, string[]>;
  registerSync: (requestId: string, subscriptions: string[]) => void;
  removeSync: (requestId: string) => void;
  clearSync: () => void;
  replace: (state: Partial<Omit<ConnectionState, 'replace'>>) => void;
}

export const useConnectionStore = create<ConnectionState>((set) => ({
  connected: false,
  ws: null,
  nickname: null,
  activeAccountId: null,
  accountGeneration: 0,
  protectedGeneration: 0,
  operationGeneration: null,
  syncCursor: null,
  syncWindowCursors: {},
  durableMode: false,
  ownPresenceStatus: null,
  ownRequestedStatus: null,
  ownCustomStatus: null,
  ownStatusEmoji: null,
  syncSubscriptions: new Map(),
  registerSync: (requestId, subscriptions) => set((state) => {
    const syncSubscriptions = new Map(state.syncSubscriptions);
    syncSubscriptions.set(requestId, subscriptions);
    return { syncSubscriptions };
  }),
  removeSync: (requestId) => set((state) => {
    const syncSubscriptions = new Map(state.syncSubscriptions);
    syncSubscriptions.delete(requestId);
    return { syncSubscriptions };
  }),
  clearSync: () => set({ syncSubscriptions: new Map() }),
  replace: (state) => set(state),
}));
