import type { ChatState, ChatStoreContext } from './types';

export function createConversationActions({ get }: ChatStoreContext): Pick<ChatState, 'pinMessage' | 'unpinMessage' | 'getPinnedMessages' | 'createThread' | 'archiveThread' | 'unarchiveThread' | 'listThreads' | 'createForumTag' | 'updateForumTag' | 'deleteForumTag' | 'listForumTags' | 'setThreadTags' | 'getThreadTags' | 'addBookmark' | 'removeBookmark' | 'listBookmarks'> {
  return {
    pinMessage: (serverId, channel, messageId) => {
      get().ws?.send({ type: 'pin_message', server_id: serverId, channel, message_id: messageId });
    },

    unpinMessage: (serverId, channel, messageId) => {
      get().ws?.send({ type: 'unpin_message', server_id: serverId, channel, message_id: messageId });
    },

    getPinnedMessages: (serverId, channel) => {
      get().ws?.send({ type: 'get_pinned_messages', server_id: serverId, channel });
    },

    createThread: (serverId, parentChannel, name, messageId, isPrivate) => {
      get().ws?.send({ type: 'create_thread', server_id: serverId, parent_channel: parentChannel, name, message_id: messageId, is_private: isPrivate });
    },

    archiveThread: (serverId, threadId) => {
      get().ws?.send({ type: 'archive_thread', server_id: serverId, thread_id: threadId });
    },

    unarchiveThread: (serverId, threadId) => {
      get().ws?.send({ type: 'unarchive_thread', server_id: serverId, thread_id: threadId });
    },

    listThreads: (serverId, channel) => {
      get().ws?.send({ type: 'list_threads', server_id: serverId, channel });
    },

    createForumTag: (serverId, channel, name, emoji, moderated) => {
      get().ws?.send({ type: 'create_forum_tag', server_id: serverId, channel, name, emoji, moderated });
    },

    updateForumTag: (serverId, channel, tag) => {
      get().ws?.send({
        type: 'update_forum_tag',
        server_id: serverId,
        channel,
        tag_id: tag.id,
        name: tag.name,
        emoji: tag.emoji,
        moderated: tag.moderated,
        position: tag.position,
      });
    },

    deleteForumTag: (serverId, channel, tagId) => {
      get().ws?.send({ type: 'delete_forum_tag', server_id: serverId, channel, tag_id: tagId });
    },

    listForumTags: (serverId, channel) => {
      get().ws?.send({ type: 'list_forum_tags', server_id: serverId, channel });
    },

    setThreadTags: (serverId, threadId, tagIds) => {
      get().ws?.send({ type: 'set_thread_tags', server_id: serverId, thread_id: threadId, tag_ids: tagIds });
    },

    getThreadTags: (serverId, threadId) => {
      get().ws?.send({ type: 'get_thread_tags', server_id: serverId, thread_id: threadId });
    },

    addBookmark: (messageId, note) => {
      get().ws?.send({ type: 'add_bookmark', message_id: messageId, note });
    },

    removeBookmark: (messageId) => {
      get().ws?.send({ type: 'remove_bookmark', message_id: messageId });
    },

    listBookmarks: () => {
      get().ws?.send({ type: 'list_bookmarks' });
    }
  };
}
