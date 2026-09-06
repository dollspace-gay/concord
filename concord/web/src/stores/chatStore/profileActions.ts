import type { ChatState, ChatStoreContext } from './types';

export function createProfileActions({ get }: ChatStoreContext): Pick<ChatState, 'setPresence' | 'getPresences' | 'setServerNickname'> {
  return {
    setPresence: (status, customStatus, statusEmoji) => {
      get().ws?.send({ type: 'set_presence', status, custom_status: customStatus, status_emoji: statusEmoji });
    },

    getPresences: (serverId) => {
      get().ws?.send({ type: 'get_presences', server_id: serverId });
    },

    setServerNickname: (serverId, nickname) => {
      get().ws?.send({ type: 'set_server_nickname', server_id: serverId, nickname });
    }
  };
}
