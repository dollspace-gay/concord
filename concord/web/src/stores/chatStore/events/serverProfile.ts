import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { ChatStoreContext } from '../types';

export function handleServerProfileEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'server_avatar_update' | 'server_limits' | 'error' }>) {
  switch (event.type) {
    case 'server_avatar_update': {
      const sa = { ...get().serverAvatars };
      if (!sa[event.server_id]) sa[event.server_id] = {};
      if (event.avatar_url) {
        sa[event.server_id] = { ...sa[event.server_id], [event.user_id]: event.avatar_url };
      } else {
        const copy = { ...sa[event.server_id] };
        delete copy[event.user_id];
        sa[event.server_id] = copy;
      }
      set({ serverAvatars: sa });
      break;
    }
    case 'server_limits': {
      set({ maxMessageLength: event.max_message_length, maxFileSizeMb: event.max_file_size_mb });
      break;
    }
    case 'error': {
      console.error(`Server error [${event.code}]: ${event.message}`);
      set({ errorToast: event.message });
      setTimeout(() => set({ errorToast: null }), 5000);
      break;
    }
  }
}
