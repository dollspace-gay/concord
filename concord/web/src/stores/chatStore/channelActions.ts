import type { ChatState, ChatStoreContext } from './types';

export function createChannelActions({ get }: ChatStoreContext): Pick<ChatState, 'joinChannel' | 'partChannel' | 'setTopic' | 'fetchHistory' | 'listChannels' | 'getMembers'> {
  return {
    joinChannel: (serverId, channel) => {
      get().ws?.send({ type: 'join_channel', server_id: serverId, channel });
    },

    partChannel: (serverId, channel) => {
      get().ws?.send({ type: 'part_channel', server_id: serverId, channel });
    },

    setTopic: (serverId, channel, topic) => {
      get().ws?.send({ type: 'set_topic', server_id: serverId, channel, topic });
    },

    fetchHistory: (serverId, channel, before) => {
      get().ws?.send({ type: 'fetch_history', server_id: serverId, channel, before, limit: 50 });
    },

    listChannels: (serverId) => {
      get().ws?.send({ type: 'list_channels', server_id: serverId });
    },

    getMembers: (serverId, channel) => {
      get().ws?.send({ type: 'get_members', server_id: serverId, channel });
    }
  };
}
