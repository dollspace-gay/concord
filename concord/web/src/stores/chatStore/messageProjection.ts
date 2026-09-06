import type { DurableMessageProjection, SnapshotReactionGroup } from '../../api/generated/contract';
import type { HistoryMessage } from '../../api/types';
import { MAX_MESSAGES_PER_CHANNEL } from './defaults';

export function durableMessage(message: DurableMessageProjection): HistoryMessage {
  return {
    id: message.message_id,
    from: message.sender_nick,
    sender_id: message.sender_id,
    sequence: message.sequence,
    deleted: message.deleted,
    content: message.deleted ? '' : (message.content ?? ''),
    timestamp: message.created_at,
    edited_at: message.edited_at,
    reply_to: message.reply_to ? {
      id: message.reply_to.message_id,
      from: message.reply_to.sender_nick,
      content_preview: message.reply_to.deleted ? '' : (message.reply_to.content ?? ''),
    } : null,
    attachments: message.deleted ? [] : message.attachments.map((attachment) => ({
      id: attachment.attachment_id,
      filename: attachment.filename,
      content_type: attachment.content_type,
      file_size: attachment.file_size,
      url: `/api/uploads/${encodeURIComponent(attachment.attachment_id)}`,
    })),
    rich_embeds: message.deleted ? null : message.rich_embeds,
    components: message.deleted ? null : message.components,
  };
}

export function mergeDurableMessage(current: HistoryMessage[], projection: DurableMessageProjection): HistoryMessage[] {
  const message = durableMessage(projection);
  const existing = current.find((candidate) => candidate.id === message.id);
  const merged = existing ? { ...existing, ...message, reactions: existing.reactions } : message;
  const next = [...current.filter((candidate) => candidate.id !== message.id), merged];
  next.sort((left, right) => left.timestamp.localeCompare(right.timestamp) || left.id.localeCompare(right.id));
  return next.length > MAX_MESSAGES_PER_CHANNEL ? next.slice(-MAX_MESSAGES_PER_CHANNEL) : next;
}

export function applyReactionProjection(
  messages: HistoryMessage[],
  reaction: { message_id: string; emoji: string; present: boolean; user_id: string },
): HistoryMessage[] {
  return messages.map((message) => {
    if (message.id !== reaction.message_id) return message;
    const groups = [...(message.reactions ?? [])];
    const index = groups.findIndex((group) => group.emoji === reaction.emoji);
    if (reaction.present) {
      if (index < 0) groups.push({ emoji: reaction.emoji, count: 1, user_ids: [reaction.user_id] });
      else if (!groups[index].user_ids.includes(reaction.user_id)) {
        const user_ids = [...groups[index].user_ids, reaction.user_id];
        groups[index] = { ...groups[index], user_ids, count: Math.max(groups[index].count + 1, user_ids.length) };
      }
    } else if (index >= 0) {
      const user_ids = groups[index].user_ids.filter((id) => id !== reaction.user_id);
      const count = Math.max(0, groups[index].count - 1);
      if (count === 0) groups.splice(index, 1);
      else groups[index] = { ...groups[index], user_ids, count };
    }
    return { ...message, reactions: groups };
  });
}

export function snapshotReactions(messageId: string, reactions: SnapshotReactionGroup[]) {
  return reactions
    .filter((reaction) => reaction.message_id === messageId)
    .map((reaction) => ({
      emoji: reaction.emoji,
      count: reaction.count,
      user_ids: reaction.reacted_by_me ? ['__self__'] : [],
    }));
}
