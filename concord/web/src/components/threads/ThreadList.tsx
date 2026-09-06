import { useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import type { ForumTagInfo, ThreadInfo } from '../../api/types';

const EMPTY_THREADS: ThreadInfo[] = [];
const EMPTY_TAGS: ForumTagInfo[] = [];

export function ThreadList() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const setActiveThreadId = useUiStore((s) => s.setActiveThreadId);
  const key = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const threads = useChatStore((s) => (key ? s.threads[key] ?? EMPTY_THREADS : EMPTY_THREADS));
  const tags = useChatStore((s) => (key ? s.forumTags[key] ?? EMPTY_TAGS : EMPTY_TAGS));
  const listThreads = useChatStore((s) => s.listThreads);
  const listForumTags = useChatStore((s) => s.listForumTags);

  useEffect(() => {
    if (activeServer && activeChannel) {
      listThreads(activeServer, activeChannel);
      listForumTags(activeServer, activeChannel);
    }
  }, [activeServer, activeChannel, listThreads, listForumTags]);

  if (threads.length === 0) {
    return null;
  }

  return (
    <section aria-label="Threads" className="border-t border-border px-3 py-2">
      <div className="mb-1 text-xs font-semibold uppercase text-text-muted">Threads</div>
      {threads.map((thread) => (
        <button
          key={thread.id}
          onClick={() => setActiveThreadId(thread.id)}
          className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left transition-colors hover:bg-bg-hover"
        >
          <svg className="h-4 w-4 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M7 8h10M7 12h4m1 8l-4-4H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-3l-4 4z" />
          </svg>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1">
              <span className="truncate text-sm text-text-primary">{thread.name}</span>
              {thread.archived && (
                <span className="rounded bg-bg-tertiary px-1 py-0.5 text-[10px] text-text-muted">Archived</span>
              )}
              {thread.channel_type === 'private_thread' && (
                <svg className="h-3 w-3 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                </svg>
              )}
            </div>
            <span className="text-xs text-text-muted">
              {thread.message_count} message{thread.message_count !== 1 ? 's' : ''}
            </span>
            {(thread.tag_ids?.length ?? 0) > 0 && (
              <div className="mt-0.5 flex flex-wrap gap-1">
                {thread.tag_ids?.map((tagId) => {
                  const tag = tags.find((candidate) => candidate.id === tagId);
                  return tag ? (
                    <span key={tag.id} className="rounded-full bg-bg-tertiary px-1.5 text-[10px] text-text-muted">
                      {tag.emoji ? `${tag.emoji} ` : ''}{tag.name}
                    </span>
                  ) : null;
                })}
              </div>
            )}
          </div>
        </button>
      ))}
    </section>
  );
}
