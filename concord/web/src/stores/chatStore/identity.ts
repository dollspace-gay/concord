import type { DirectConversationInfo } from '../../api/generated/contract';
import type { ChannelInfo } from '../../api/types';
import { channelKey } from '../../api/types';

/** Cache an avatar_url for a nickname if present. */
export function cacheAvatar(avatars: Record<string, string>, nickname: string, avatar_url?: string | null): Record<string, string> {
  if (avatar_url && avatars[nickname] !== avatar_url) {
    return { ...avatars, [nickname]: avatar_url };
  }
  return avatars;
}

export function conversationKey(
  channels: Record<string, ChannelInfo[]>,
  directConversations: DirectConversationInfo[],
  conversationId: string,
): string | null {
  for (const [serverId, entries] of Object.entries(channels)) {
    const channel = entries.find((candidate) => candidate.conversation_id === conversationId);
    if (channel) return channelKey(serverId, channel.name);
  }
  if (directConversations.some((conversation) => conversation.id === conversationId)) {
    return `dm:${conversationId}`;
  }
  return null;
}

export function entityVersionKey(entityType: string, entityId: string): string {
  return `${entityType}:${entityId}`;
}
