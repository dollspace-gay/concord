import type { DirectConversationInfo } from '../../api/generated/contract';
import type { ChannelInfo } from '../../api/types';
import { WebSocketManager } from '../../api/websocket';
import { useConnectionStore } from '../connectionStore';
import { usePendingStore } from '../pendingStore';
import { UNCERTAIN_LIFECYCLE_MESSAGE } from './defaults';

export function currentSubscriptions(
  channels: Record<string, ChannelInfo[]>,
  directConversations: DirectConversationInfo[] = [],
): string[] {
  return [...new Set([
    ...Object.values(channels).flat().map((channel) => channel.conversation_id),
    ...directConversations.map((conversation) => conversation.id),
  ])].sort();
}

export function requestSync(
  ws: WebSocketManager,
  subscriptions: string[],
  cursor?: string,
  windowCursors: Record<string, string> = {},
): void {
  const windows = subscriptions.length
    ? Array.from({ length: Math.ceil(subscriptions.length / 100) }, (_, index) =>
      subscriptions.slice(index * 100, (index + 1) * 100))
    : [[]];
  for (const window of windows) {
    const windowId = window.join('\u0000');
    const resumeCursor = cursor ?? windowCursors[windowId];
    const requestId = crypto.randomUUID();
    useConnectionStore.getState().registerSync(requestId, window);
    ws.send({
      type: 'sync',
      request_id: requestId,
      protocol_version: 2,
      subscriptions: window,
      ...(resumeCursor ? { cursor: resumeCursor } : {}),
      limit: 100,
    });
  }
}

export function rejectPendingInteractions(reason: string) {
  usePendingStore.getState().rejectAllInteractions(reason);
  usePendingStore.getState().rejectAllLifecycle(UNCERTAIN_LIFECYCLE_MESSAGE);
}
