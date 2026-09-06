import type { ChatState, ChatStoreContext } from './types';

export function createServerActions({ get }: ChatStoreContext): Pick<ChatState, 'listServers' | 'createServer' | 'joinServer' | 'leaveServer' | 'createChannel' | 'deleteChannel' | 'deleteServer' | 'updateServer'> {
  return {
    listServers: () => {
      get().ws?.send({ type: 'list_servers' });
    },

    createServer: (name, iconUrl) => {
      return get().runLifecycleCommand(
        { type: 'create_server', name, icon_url: iconUrl },
        'server:create',
      );
    },

    joinServer: (serverId) => {
      get().ws?.send({ type: 'join_server', server_id: serverId });
    },

    leaveServer: (serverId) => {
      get().ws?.send({ type: 'leave_server', server_id: serverId });
    },

    createChannel: (serverId, name, categoryId, isPrivate, channelType) => {
      get().ws?.send({ type: 'create_channel', server_id: serverId, name, category_id: categoryId, is_private: isPrivate, channel_type: channelType });
    },

    deleteChannel: (serverId, channel) => {
      get().ws?.send({ type: 'delete_channel', server_id: serverId, channel });
    },

    deleteServer: (serverId) => {
      return get().runLifecycleCommand(
        { type: 'delete_server', server_id: serverId },
        `server:${serverId}:delete`,
      );
    },

    updateServer: (serverId, name, iconUrl) => {
      return get().runLifecycleCommand(
        { type: 'update_server', server_id: serverId, name, icon_url: iconUrl },
        `server:${serverId}:update`,
      );
    }
  };
}
