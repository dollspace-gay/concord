import type { ChatState, ChatStoreContext } from './types';

export function createNotificationActions({ get }: ChatStoreContext): Pick<ChatState, 'updateNotificationSettings' | 'getNotificationSettings' | 'getUserProfile'> {
  return {
    updateNotificationSettings: (serverId, channelId, level, options) => {
      get().ws?.send({
        type: 'update_notification_settings',
        server_id: serverId,
        channel_id: channelId,
        level,
        suppress_everyone: options?.suppressEveryone,
        suppress_roles: options?.suppressRoles,
        muted: options?.muted,
        mute_until: options?.muteUntil,
      });
    },

    getNotificationSettings: (serverId) => {
      get().ws?.send({ type: 'get_notification_settings', server_id: serverId });
    },

    getUserProfile: (userId) => {
      get().ws?.send({ type: 'get_user_profile', user_id: userId });
    }
  };
}
