import { createServerSticker, deleteServerSticker, listAllUserEmoji, listServerStickers } from '../../api/client';
import type { ChatState, ChatStoreContext } from './types';

export function createMediaActions({ set, get }: ChatStoreContext): Pick<ChatState, 'loadServerStickers' | 'createSticker' | 'deleteSticker' | 'loadAllUserEmoji' | 'setServerAvatar' | 'setVanityCode'> {
  return {
    // ── Phase 9.5: Premium-for-Free Features ──
    loadServerStickers: (serverId) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      listServerStickers(serverId)
        .then((stickers) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          set((s) => ({ stickers: { ...s.stickers, [serverId]: stickers } }));
        })
        .catch((e) => console.error('Failed to load stickers:', e));
    },

    createSticker: async (serverId, name, imageUrl, description) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      await createServerSticker(serverId, name, imageUrl, description);
      if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
      // Reload stickers for this server
      listServerStickers(serverId)
        .then((stickers) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          set((s) => ({ stickers: { ...s.stickers, [serverId]: stickers } }));
        })
        .catch((e) => console.error('Failed to reload stickers:', e));
    },

    deleteSticker: async (serverId, stickerId) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      await deleteServerSticker(serverId, stickerId);
      if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
      set((s) => ({
        stickers: {
          ...s.stickers,
          [serverId]: (s.stickers[serverId] || []).filter((st) => st.id !== stickerId),
        },
      }));
    },

    loadAllUserEmoji: (targetServerId) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      listAllUserEmoji(targetServerId)
        .then((emoji) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          set({ allUserEmoji: emoji });
        })
        .catch((e) => console.error('Failed to load cross-server emoji:', e));
    },

    setServerAvatar: (serverId, avatarUrl) => {
      get().ws?.send({ type: 'set_server_avatar', server_id: serverId, avatar_url: avatarUrl ?? null });
    },

    setVanityCode: (serverId, vanityCode) => {
      return get().runLifecycleCommand(
        { type: 'set_vanity_code', server_id: serverId, vanity_code: vanityCode ?? null },
        `community:${serverId}:vanity`,
      );
    }
  };
}
