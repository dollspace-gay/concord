import { create } from 'zustand';
import type { ClientMessage } from '../api/generated/contract';

const MAX_PENDING_LIFECYCLE_COMMANDS = 128;

interface PendingState {
  pendingCommands: Record<string, ClientMessage>;
  interactions: Record<string, PendingInteraction>;
  lifecycleCommands: Record<string, PendingLifecycleCommand>;
  lifecycleKeys: Record<string, true>;
  replace: (pendingCommands: Record<string, ClientMessage>) => void;
  registerInteraction: (requestId: string, pending: PendingInteraction) => void;
  takeInteraction: (requestId: string) => PendingInteraction | undefined;
  rejectAllInteractions: (reason: string) => void;
  registerLifecycle: (requestId: string, pending: PendingLifecycleCommand) => boolean;
  takeLifecycle: (requestId: string) => PendingLifecycleCommand | undefined;
  rejectAllLifecycle: (reason: string) => void;
}

export interface PendingInteraction {
  accountGeneration: number;
  resolve: () => void;
  reject: (error: Error) => void;
}

export interface PendingLifecycleCommand {
  accountGeneration: number;
  connection: object;
  key: string;
  deadline: ReturnType<typeof setTimeout>;
  resolve: () => void;
  reject: (error: Error) => void;
}

export class LifecycleOutcomeUncertainError extends Error {
  readonly uncertain = true;
}

export const usePendingStore = create<PendingState>((set, get) => ({
  pendingCommands: {},
  interactions: {},
  lifecycleCommands: {},
  lifecycleKeys: {},
  replace: (pendingCommands) => set({ pendingCommands }),
  registerInteraction: (requestId, pending) => set((state) => ({
    interactions: { ...state.interactions, [requestId]: pending },
  })),
  takeInteraction: (requestId) => {
    const pending = get().interactions[requestId];
    if (pending) set((state) => ({
      interactions: Object.fromEntries(Object.entries(state.interactions).filter(([id]) => id !== requestId)),
    }));
    return pending;
  },
  rejectAllInteractions: (reason) => set((state) => {
    for (const pending of Object.values(state.interactions)) pending.reject(new Error(reason));
    return { interactions: {} };
  }),
  registerLifecycle: (requestId, pending) => {
    if (get().lifecycleKeys[pending.key]
        || Object.keys(get().lifecycleCommands).length >= MAX_PENDING_LIFECYCLE_COMMANDS) return false;
    set((state) => ({
      lifecycleCommands: { ...state.lifecycleCommands, [requestId]: pending },
      lifecycleKeys: { ...state.lifecycleKeys, [pending.key]: true },
    }));
    return true;
  },
  takeLifecycle: (requestId) => {
    const pending = get().lifecycleCommands[requestId];
    if (pending) set((state) => {
      clearTimeout(pending.deadline);
      const lifecycleCommands = { ...state.lifecycleCommands };
      const lifecycleKeys = { ...state.lifecycleKeys };
      delete lifecycleCommands[requestId];
      delete lifecycleKeys[pending.key];
      return { lifecycleCommands, lifecycleKeys };
    });
    return pending;
  },
  rejectAllLifecycle: (reason) => set((state) => {
    for (const pending of Object.values(state.lifecycleCommands)) {
      clearTimeout(pending.deadline);
      pending.reject(new LifecycleOutcomeUncertainError(reason));
    }
    return { lifecycleCommands: {}, lifecycleKeys: {} };
  }),
}));
