import type { ChatState, ChatStoreContext } from './types';

export function createAnnouncementActions({ get }: ChatStoreContext): Pick<ChatState, 'setAnnouncementChannel' | 'followChannel' | 'unfollowChannel' | 'listChannelFollows'> {
  return {
    setAnnouncementChannel: (serverId, channel, isAnnouncement) => {
      return get().runLifecycleCommand(
        { type: 'set_announcement_channel', server_id: serverId, channel, is_announcement: isAnnouncement },
        `community:${serverId}:announcement:${channel}`,
      );
    },

    followChannel: (sourceChannelId, targetChannelId) => {
      return get().runLifecycleCommand(
        { type: 'follow_channel', source_channel_id: sourceChannelId, target_channel_id: targetChannelId },
        `community:follow:${sourceChannelId}:${targetChannelId}`,
      );
    },

    unfollowChannel: (followId) => {
      return get().runLifecycleCommand(
        { type: 'unfollow_channel', follow_id: followId },
        `community:follow:${followId}`,
      );
    },

    listChannelFollows: (channelId) => {
      get().ws?.send({ type: 'list_channel_follows', channel_id: channelId });
    }
  };
}
