import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { ChatStoreContext } from '../types';

export function handleBookmarksEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'bookmark_list' | 'bookmark_add' | 'bookmark_remove' }>) {
  switch (event.type) {
    case 'bookmark_list': {
      set({ bookmarks: event.bookmarks });
      break;
    }
    case 'bookmark_add': {
      set((s) => ({
        bookmarks: [...s.bookmarks, event.bookmark],
      }));
      break;
    }
    case 'bookmark_remove': {
      set((s) => ({
        bookmarks: s.bookmarks.filter((b) => b.message_id !== event.message_id),
      }));
      break;
    }
  }
}
