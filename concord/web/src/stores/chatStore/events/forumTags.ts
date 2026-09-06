import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import { entityVersionKey } from '../identity';
import type { ChatStoreContext } from '../types';

export function handleForumTagsEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'forum_tag_list' | 'forum_tag_update' | 'forum_tag_delete' | 'thread_tag_update' }>) {
  switch (event.type) {
    case 'forum_tag_list': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => ({
        forumTags: { ...s.forumTags, [key]: event.tags },
      }));
      break;
    }
    case 'forum_tag_update': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => {
        const current = s.forumTags[key] || [];
        const idx = current.findIndex((t) => t.id === event.tag.id);
        const updated = idx >= 0
          ? current.map((t) => (t.id === event.tag.id ? event.tag : t))
          : [...current, event.tag];
        return { forumTags: { ...s.forumTags, [key]: updated } };
      });
      break;
    }
    case 'forum_tag_delete': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => ({
        forumTags: {
          ...s.forumTags,
          [key]: (s.forumTags[key] || []).filter((t) => t.id !== event.tag_id),
        },
      }));
      break;
    }
    case 'thread_tag_update': {
      set((s) => {
        const versionKey = entityVersionKey('thread_tags', event.thread_id);
        if ((s.entityVersions[versionKey] ?? 0) >= event.version) return {};
        return {
          threads: Object.fromEntries(Object.entries(s.threads).map(([parent, entries]) => [
            parent,
            entries.map((thread) => thread.id === event.thread_id
              ? { ...thread, tag_ids: event.tag_ids, tags_version: event.version }
              : thread),
          ])),
          entityVersions: { ...s.entityVersions, [versionKey]: event.version },
        };
      });
      break;
    }
  }
}
