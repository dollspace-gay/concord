import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import { MAX_MESSAGES_PER_CHANNEL } from '../defaults';
import type { ChatStoreContext } from '../types';

export function handleHistoryEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'history' }>) {
  switch (event.type) {
    case 'history': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => {
        const current = s.messages[key] || [];
        const combined = s.durableMode
          ? [...current, ...event.messages.reverse()]
          : [...event.messages.reverse(), ...current];
        // Deduplicate by message ID
        const seen = new Set<string>();
        const deduped = combined.filter(m => {
          if (seen.has(m.id)) return false;
          seen.add(m.id);
          return true;
        });
        const trimmed = deduped.length > MAX_MESSAGES_PER_CHANNEL
          ? deduped.slice(deduped.length - MAX_MESSAGES_PER_CHANNEL)
          : deduped;
        return {
          messages: {
            ...s.messages,
            [key]: trimmed,
          },
          hasMore: { ...s.hasMore, [key]: event.has_more },
        };
      });
      break;
    }
  }
}
