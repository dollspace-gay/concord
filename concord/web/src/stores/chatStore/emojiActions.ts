import { createServerEmoji, deleteServerEmoji, listServerEmoji } from '../../api/client';
import type { ChatState, ChatStoreContext } from './types';

export function createEmojiActions({ set, get }: ChatStoreContext): Pick<ChatState, 'loadServerEmoji' | 'createEmoji' | 'deleteEmoji'> {
  return {
    loadServerEmoji: (serverId) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      return listServerEmoji(serverId)
        .then((emojis) => {
          if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
          const map: Record<string, { id: string; image_url: string }> = {};
          for (const e of emojis) {
            map[e.name] = { id: e.id, image_url: e.image_url };
          }
          set((s) => ({
            customEmoji: { ...s.customEmoji, [serverId]: map },
          }));
        })
        .catch((err) => {
          console.error('Failed to load emoji for server', serverId, err);
        });
    },

    createEmoji: async (serverId, name, imageUrl) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      await createServerEmoji(serverId, name, imageUrl);
      if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
      get().loadServerEmoji(serverId);
    },

    deleteEmoji: async (serverId, emojiId) => {
      const generation = get().protectedGeneration;
      const accountId = get().activeAccountId;
      await deleteServerEmoji(serverId, emojiId);
      if (get().protectedGeneration !== generation || get().activeAccountId !== accountId) return;
      get().loadServerEmoji(serverId);
    }
  };
}
