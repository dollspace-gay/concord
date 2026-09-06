import type { HistoryMessage } from '../../api/types';
import type { ChatState } from './types';

export function removeDeletedReferences(state: ChatState, deleted: Set<string>) {
  return {
    pinnedMessages: Object.fromEntries(Object.entries(state.pinnedMessages).map(([key, pins]) => [
      key, pins.filter((pin) => !deleted.has(pin.message_id)),
    ])),
    bookmarks: state.bookmarks.filter((bookmark) => !deleted.has(bookmark.message_id)),
    searchResults: state.searchResults?.filter((result) => !deleted.has(result.id)) ?? null,
  };
}

export function withoutServers<T>(record: Record<string, T>, retained: Set<string>): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([serverId]) => retained.has(serverId)));
}

export function withoutChannelKeys<T>(record: Record<string, T>, removed: Set<string>): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([key]) => {
    const separator = key.indexOf(':');
    return separator < 0 || !removed.has(key.slice(0, separator));
  }));
}

export function redactDeletedReplyPreviews(messages: HistoryMessage[], deleted: Set<string>) {
  return messages.map((message) => message.reply_to && deleted.has(message.reply_to.id)
    ? { ...message, reply_to: { id: message.reply_to.id, from: message.reply_to.from, content_preview: '' } }
    : message);
}
