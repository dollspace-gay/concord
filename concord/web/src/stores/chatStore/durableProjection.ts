import type { DurableEventProjection } from '../../api/generated/contract';
import { conversationKey, entityVersionKey } from './identity';
import { applyReactionProjection, mergeDurableMessage } from './messageProjection';
import { redactDeletedReplyPreviews, removeDeletedReferences } from './references';
import type { ChatState } from './types';

export function durableProjectionUpdate(
  projection: DurableEventProjection,
  state: ChatState,
): Partial<ChatState> {
  const key = conversationKey(state.channels, state.directConversations, projection.conversation_id);
  if (!key) return {};
  const versionKey = entityVersionKey(projection.entity_type, projection.entity_id);
  if ((state.entityVersions[versionKey] ?? 0) >= projection.entity_version) return {};
  const entityVersions = {
    ...state.entityVersions,
    [versionKey]: projection.entity_version,
  };
  if (projection.message) {
    const current = state.messages[key] ?? [];
    const isNewVisibleMessage = !projection.message.deleted
      && !current.some((message) => message.id === projection.message?.message_id)
      && projection.message.sender_id !== state.activeAccountId
      && BigInt(projection.message.sequence) > BigInt(state.readSequences[projection.conversation_id] ?? '0');
    const mergedMessages = mergeDurableMessage(current, projection.message);
    const deleted = new Set([projection.message.message_id]);
    const messages = projection.message.deleted
      ? redactDeletedReplyPreviews(mergedMessages, deleted)
      : mergedMessages;
    return {
      messages: {
        ...state.messages,
        [key]: messages,
      },
      unreadCounts: isNewVisibleMessage
        ? { ...state.unreadCounts, [key]: (state.unreadCounts[key] ?? 0) + 1 }
        : state.unreadCounts,
      entityVersions,
      ...(projection.message.deleted ? {
        ...removeDeletedReferences(state, deleted),
        deletedMessageIds: { ...state.deletedMessageIds, [projection.message.message_id]: true },
      } : {}),
    };
  }
  if (projection.reaction) {
    return {
      messages: {
        ...state.messages,
        [key]: applyReactionProjection(state.messages[key] ?? [], projection.reaction),
      },
      entityVersions,
    };
  }
  if (projection.read_state) {
    const previousSequence = state.readSequences[projection.conversation_id] ?? '0';
    if (BigInt(projection.read_state.sequence) <= BigInt(previousSequence)) {
      return { entityVersions };
    }
    const unread = (state.messages[key] ?? []).filter((message) =>
      !message.deleted
      && message.sender_id !== state.activeAccountId
      && BigInt(message.sequence ?? '0') > BigInt(projection.read_state!.sequence)).length;
    return {
      unreadCounts: { ...state.unreadCounts, [key]: unread },
      readSequences: {
        ...state.readSequences,
        [projection.conversation_id]: projection.read_state.sequence,
      },
      entityVersions,
    };
  }
  if (projection.entity_type === 'thread_state'
    && projection.kind === 'thread_state_changed'
    && typeof projection.descriptor === 'object'
    && projection.descriptor !== null
    && 'archived' in projection.descriptor
    && typeof projection.descriptor.archived === 'boolean') {
    const archived = projection.descriptor.archived;
    const channels = Object.fromEntries(Object.entries(state.channels).map(([serverId, entries]) => [
      serverId,
      entries.map((channel) => channel.id === projection.entity_id ? { ...channel, archived } : channel),
    ]));
    const threads = Object.fromEntries(Object.entries(state.threads).map(([parent, entries]) => [
      parent,
      entries.map((thread) => thread.id === projection.entity_id ? { ...thread, archived } : thread),
    ]));
    return { channels, threads, entityVersions };
  }
  if (projection.entity_type === 'thread_tags'
    && projection.kind === 'thread_tags_updated'
    && typeof projection.descriptor === 'object'
    && projection.descriptor !== null
    && 'thread_id' in projection.descriptor
    && typeof projection.descriptor.thread_id === 'string'
    && 'tag_ids' in projection.descriptor
    && Array.isArray(projection.descriptor.tag_ids)
    && projection.descriptor.tag_ids.every((tag): tag is string => typeof tag === 'string')) {
    const threadId = projection.descriptor.thread_id;
    const tagIds = [...projection.descriptor.tag_ids];
    const threads = Object.fromEntries(Object.entries(state.threads).map(([parent, entries]) => [
      parent,
      entries.map((thread) => thread.id === threadId
        ? { ...thread, tag_ids: tagIds, tags_version: projection.entity_version }
        : thread),
    ]));
    return { threads, entityVersions };
  }
  return {};
}
