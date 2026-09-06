import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import type { ChatStoreContext } from '../types';

export function handlePinsEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'message_pin' | 'message_unpin' | 'pinned_messages' }>) {
  switch (event.type) {
    case 'message_pin': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => ({
        pinnedMessages: {
          ...s.pinnedMessages,
          [key]: [...(s.pinnedMessages[key] || []), event.pin],
        },
      }));
      break;
    }
    case 'message_unpin': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => ({
        pinnedMessages: {
          ...s.pinnedMessages,
          [key]: (s.pinnedMessages[key] || []).filter((p) => p.message_id !== event.message_id),
        },
      }));
      break;
    }
    case 'pinned_messages': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => ({
        pinnedMessages: { ...s.pinnedMessages, [key]: event.pins },
      }));
      break;
    }
  }
}
