import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { HistoryMessage } from '../../../api/types';
import { useComposerStore } from '../../composerStore';
import { useConnectionStore } from '../../connectionStore';
import { durableProjectionUpdate } from '../durableProjection';
import { conversationKey, entityVersionKey } from '../identity';
import { durableMessage, snapshotReactions } from '../messageProjection';
import { maybeNotifyMessage } from '../notifications';
import { protectedStateReset } from '../reset';
import { hydratedServerMetadata, pendingCommandOwners, recoveredThisConnection } from '../runtime';
import { requestSync } from '../synchronization';
import type { ChatStoreContext } from '../types';

export function handleSyncEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'sync_snapshot' | 'replay_batch' | 'durable_event' | 'resync_required' }>) {
  switch (event.type) {
    case 'sync_snapshot': {
      const syncSubscriptions = useConnectionStore.getState().syncSubscriptions;
      const correlated = syncSubscriptions.get(event.request_id);
      if (!correlated && (get().durableMode || syncSubscriptions.size > 0)) break;
      const subscriptions = correlated ?? [];
      const windowId = subscriptions.join('\u0000');
      set((state) => {
        const replacedKeys = new Set(subscriptions
          .map((conversation) => conversationKey(state.channels, state.directConversations, conversation))
          .filter((key): key is string => key !== null));
        const replacedMessageIds = new Set(Object.entries(state.messages)
          .filter(([key]) => replacedKeys.has(key))
          .flatMap(([, entries]) => entries.map((message) => message.id)));
        const messages: Record<string, HistoryMessage[]> = Object.fromEntries(
          Object.entries(state.messages).filter(([key]) => !replacedKeys.has(key)),
        );
        const unreadCounts: Record<string, number> = Object.fromEntries(
          Object.entries(state.unreadCounts).filter(([key]) => !replacedKeys.has(key)),
        );
        const hasMore: Record<string, boolean> = Object.fromEntries(
          Object.entries(state.hasMore).filter(([key]) => !replacedKeys.has(key)),
        );
        const entityVersions: Record<string, number> = Object.fromEntries(
          Object.entries(state.entityVersions).filter(([entity]) =>
            ![...replacedMessageIds].some((messageId) => entity === entityVersionKey('message', messageId))
            && !subscriptions.some((conversation) => entity === entityVersionKey('read_state', conversation))),
        );
        const snapshotReadSequences = new Map(
          event.snapshot.read_states.map((read) => [read.conversation_id, BigInt(read.sequence)]),
        );
        const readSequences = Object.fromEntries(Object.entries(state.readSequences)
          .filter(([conversation]) => !subscriptions.includes(conversation)));
        for (const projection of event.snapshot.messages) {
          entityVersions[entityVersionKey('message', projection.message_id)] = projection.entity_version;
          const key = conversationKey(state.channels, state.directConversations, projection.conversation_id);
          if (!key) continue;
          const mapped = durableMessage(projection);
          mapped.reactions = snapshotReactions(projection.message_id, event.snapshot.reactions);
          messages[key] = [...(messages[key] ?? []), mapped];
          if (!projection.deleted
            && projection.sender_id !== state.activeAccountId
            && BigInt(projection.sequence) > (snapshotReadSequences.get(projection.conversation_id) ?? 0n)) {
            unreadCounts[key] = (unreadCounts[key] ?? 0) + 1;
          }
          hasMore[key] = event.snapshot.history_before[projection.conversation_id] !== undefined;
        }
        for (const read of event.snapshot.read_states) {
          entityVersions[entityVersionKey('read_state', read.conversation_id)] = read.entity_version;
          readSequences[read.conversation_id] = read.sequence;
        }
        return {
          operationGeneration: event.snapshot.operation_generation,
          syncCursor: event.snapshot.cursor,
          syncWindowCursors: { ...state.syncWindowCursors, [windowId]: event.snapshot.cursor },
          messages,
          unreadCounts,
          hasMore,
          durableMode: true,
          entityVersions,
          readSequences,
        };
      });
      useConnectionStore.getState().removeSync(event.request_id);
      const { accountGeneration, pendingCommands, ws } = get();
      let recovered = 0;
      for (const [requestId, command] of Object.entries(pendingCommands)) {
        if (pendingCommandOwners.get(requestId) === accountGeneration
          && !recoveredThisConnection.has(requestId)
          && ws?.send(command)) {
          recoveredThisConnection.add(requestId);
          recovered += 1;
        }
      }
      if (recovered > 0) {
        set({ errorToast: `Restoring ${recovered} pending ${recovered === 1 ? 'change' : 'changes'}…` });
        setTimeout(() => set({ errorToast: null }), 5000);
      }
      break;
    }
    case 'replay_batch': {
      const correlated = useConnectionStore.getState().syncSubscriptions.get(event.request_id);
      if (!correlated) break;
      const subscriptions = correlated;
      const windowId = subscriptions.join('\u0000');
      for (const projection of event.batch.events) set((state) => durableProjectionUpdate(projection, state));
      set((state) => ({
        operationGeneration: event.batch.operation_generation,
        syncCursor: event.batch.cursor,
        syncWindowCursors: { ...state.syncWindowCursors, [windowId]: event.batch.cursor },
      }));
      if (event.batch.has_more) {
        useConnectionStore.getState().removeSync(event.request_id);
        if (get().ws) requestSync(get().ws!, subscriptions, event.batch.cursor, get().syncWindowCursors);
      } else useConnectionStore.getState().removeSync(event.request_id);
      break;
    }
    case 'durable_event': {
      if (event.event.message && !event.event.message.deleted) {
        const state = get();
        const key = conversationKey(state.channels, state.directConversations, event.event.conversation_id);
        const isNew = key && !(state.messages[key] ?? []).some((message) => message.id === event.event.message?.message_id);
        if (isNew) {
          const channelEntry = Object.entries(state.channels)
            .flatMap(([serverId, channels]) => channels.map((channel) => ({ serverId, channel })))
            .find(({ channel }) => channel.conversation_id === event.event.conversation_id);
          maybeNotifyMessage(get, state, {
            id: event.event.message.message_id,
            senderId: event.event.message.sender_id,
            senderNick: event.event.message.sender_nick,
            content: event.event.message.content,
            mentions: event.event.message.mentions,
          }, channelEntry?.serverId, channelEntry?.channel.id);
        }
      }
      set((state) => durableProjectionUpdate(event.event, state));
      break;
    }
    case 'resync_required': {
      const serverIds = get().servers.map((server) => server.id);
      hydratedServerMetadata.clear();
      useConnectionStore.getState().clearSync();
      useComposerStore.getState().setReplyingTo(null);
      set({
        ...protectedStateReset(),
        protectedGeneration: get().protectedGeneration + 1,
        operationGeneration: null,
        syncCursor: null,
        syncWindowCursors: {},
        durableMode: true,
        entityVersions: {},
      });
      get().ws?.send({ type: 'list_servers' });
      for (const serverId of serverIds) get().ws?.send({ type: 'list_channels', server_id: serverId });
      if (get().ws) requestSync(get().ws!, []);
      break;
    }
  }
}
