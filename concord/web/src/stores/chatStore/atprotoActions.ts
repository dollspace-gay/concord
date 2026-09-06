import { getAtprotoSyncSetting, getBlueskyIdentity, getServerLimits, shareToBluesky, syncBlueskyProfile, updateAtprotoSyncSetting } from '../../api/client';
import type { ChatState, ChatStoreContext } from './types';

export function createAtprotoActions({ set, get }: ChatStoreContext): Pick<ChatState, 'fetchServerLimits' | 'syncBlueskyProfile' | 'fetchBlueskyIdentity' | 'shareToBluesky' | 'fetchAtprotoSyncSetting' | 'setAtprotoSyncEnabled'> {
  return {
    fetchServerLimits: () => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      getServerLimits()
        .then((limits) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          set({ maxMessageLength: limits.max_message_length, maxFileSizeMb: limits.max_file_size_mb });
        })
        .catch((e) => console.error('Failed to fetch server limits:', e));
    },

    // ── Phase 9: AT Protocol Deep Integration ──
    syncBlueskyProfile: async () => {
      await syncBlueskyProfile();
    },

    fetchBlueskyIdentity: (userId) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      getBlueskyIdentity(userId)
        .then((identity) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          set((s) => ({
            blueskyIdentities: { ...s.blueskyIdentities, [userId]: identity },
          }));
        })
        .catch(() => { /* user may not have Bluesky linked */ });
    },

    shareToBluesky: async (messageId) => {
      return await shareToBluesky(messageId);
    },

    fetchAtprotoSyncSetting: () => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      getAtprotoSyncSetting()
        .then((r) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          set({ atprotoSyncEnabled: r.atproto_sync_enabled });
        })
        .catch(() => { /* not authenticated or no AT Protocol account */ });
    },

    setAtprotoSyncEnabled: async (enabled) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      const r = await updateAtprotoSyncSetting(enabled);
      if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
      set({ atprotoSyncEnabled: r.atproto_sync_enabled });
    }
  };
}
