import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { ChatStoreContext } from '../types';

export function handleCategoriesEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'category_list' | 'category_update' | 'category_delete' | 'channel_reorder' }>) {
  switch (event.type) {
    case 'category_list': {
      set((s) => ({
        categories: { ...s.categories, [event.server_id]: event.categories },
      }));
      break;
    }
    case 'category_update': {
      set((s) => {
        const current = s.categories[event.server_id] || [];
        const idx = current.findIndex((c) => c.id === event.category.id);
        const updated = idx >= 0
          ? current.map((c) => (c.id === event.category.id ? event.category : c))
          : [...current, event.category];
        return { categories: { ...s.categories, [event.server_id]: updated } };
      });
      break;
    }
    case 'category_delete': {
      set((s) => ({
        categories: {
          ...s.categories,
          [event.server_id]: (s.categories[event.server_id] || []).filter((c) => c.id !== event.category_id),
        },
      }));
      break;
    }
    case 'channel_reorder': {
      set((s) => {
        const channels = s.channels[event.server_id];
        if (!channels) return s;
        const updated = channels.map((ch) => {
          const pos = event.channels.find((p) => p.id === ch.id);
          if (pos) {
            return { ...ch, position: pos.position, category_id: pos.category_id };
          }
          return ch;
        });
        return { channels: { ...s.channels, [event.server_id]: updated } };
      });
      break;
    }
  }
}
