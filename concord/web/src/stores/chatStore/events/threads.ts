import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import { entityVersionKey } from '../identity';
import type { ChatStoreContext } from '../types';

export function handleThreadsEvents({ set }: ChatStoreContext, event: Extract<ServerEvent, { type: 'thread_create' | 'thread_update' | 'thread_list' }>) {
  switch (event.type) {
    case 'thread_create': {
      const key = channelKey(event.server_id, event.parent_channel);
      set((s) => ({
        threads: {
          ...s.threads,
          [key]: [...(s.threads[key] || []), event.thread],
        },
      }));
      break;
    }
    case 'thread_update': {
      set((s) => {
        const newThreads = { ...s.threads };
        for (const ch in newThreads) {
          const idx = newThreads[ch].findIndex((t) => t.id === event.thread.id);
          if (idx >= 0) {
            newThreads[ch] = newThreads[ch].map((t) =>
              t.id === event.thread.id ? event.thread : t,
            );
            break;
          }
        }
        return { threads: newThreads };
      });
      break;
    }
    case 'thread_list': {
      const key = channelKey(event.server_id, event.channel);
      set((s) => ({
        threads: { ...s.threads, [key]: event.threads },
        entityVersions: event.threads.reduce((versions, thread) => ({
          ...versions,
          [entityVersionKey('thread_state', thread.id)]: Math.max(
            versions[entityVersionKey('thread_state', thread.id)] ?? 0,
            thread.state_version ?? 0,
          ),
          [entityVersionKey('thread_tags', thread.id)]: Math.max(
            versions[entityVersionKey('thread_tags', thread.id)] ?? 0,
            thread.tags_version ?? 0,
          ),
        }), s.entityVersions),
      }));
      break;
    }
  }
}
