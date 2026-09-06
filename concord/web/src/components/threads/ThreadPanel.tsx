import { useState, useEffect, useRef } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import type { ForumTagInfo, HistoryMessage, ThreadInfo } from '../../api/types';
import { FormattedMessage } from '../chat/FormattedMessage';

const EMPTY_MESSAGES: HistoryMessage[] = [];
const EMPTY_THREADS: ThreadInfo[] = [];
const EMPTY_FORUM_TAGS: ForumTagInfo[] = [];

export function ThreadPanel() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const activeThreadId = useUiStore((s) => s.activeThreadId);
  const setActiveThreadId = useUiStore((s) => s.setActiveThreadId);
  const setShowThreadPanel = useUiStore((s) => s.setShowThreadPanel);

  // Find the thread info from the threads store
  const parentKey = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const threads = useChatStore((s) => (parentKey ? s.threads[parentKey] ?? EMPTY_THREADS : EMPTY_THREADS));
  const thread = threads.find((t) => t.id === activeThreadId) || null;

  // Thread messages use the thread name as channel key
  const threadKey = activeServer && thread ? channelKey(activeServer, thread.name) : null;
  const messages = useChatStore((s) => (threadKey ? s.messages[threadKey] ?? EMPTY_MESSAGES : EMPTY_MESSAGES));
  const fetchHistory = useChatStore((s) => s.fetchHistory);
  const sendMessage = useChatStore((s) => s.sendMessage);
  const archiveThread = useChatStore((s) => s.archiveThread);
  const unarchiveThread = useChatStore((s) => s.unarchiveThread);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const forumTags = useChatStore((s) => (parentKey ? s.forumTags[parentKey] ?? EMPTY_FORUM_TAGS : EMPTY_FORUM_TAGS));
  const listForumTags = useChatStore((s) => s.listForumTags);
  const getThreadTags = useChatStore((s) => s.getThreadTags);
  const setThreadTags = useChatStore((s) => s.setThreadTags);
  const avatars = useChatStore((s) => s.avatars);

  const [input, setInput] = useState('');
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Join thread channel and fetch history on mount
  useEffect(() => {
    if (activeServer && thread) {
      joinChannel(activeServer, thread.name);
      fetchHistory(activeServer, thread.name);
      getThreadTags(activeServer, thread.id);
      if (activeChannel) listForumTags(activeServer, activeChannel);
    }
  }, [activeServer, activeChannel, thread, joinChannel, fetchHistory, getThreadTags, listForumTags]);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages.length]);

  const handleSend = () => {
    const trimmed = input.trim();
    if (!trimmed || !activeServer || !thread) return;
    sendMessage(activeServer, thread.name, trimmed);
    setInput('');
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleArchive = () => {
    if (activeServer && activeThreadId) {
      archiveThread(activeServer, activeThreadId);
    }
  };

  const handleUnarchive = () => {
    if (activeServer && activeThreadId) {
      unarchiveThread(activeServer, activeThreadId);
    }
  };

  const handleClose = () => {
    setActiveThreadId(null);
    setShowThreadPanel(false);
  };

  const toggleTag = (tagId: string) => {
    if (!activeServer || !thread) return;
    const selected = thread.tag_ids ?? [];
    const next = selected.includes(tagId)
      ? selected.filter((id) => id !== tagId)
      : [...selected, tagId];
    if (next.length <= 5) setThreadTags(activeServer, thread.id, next);
  };

  if (!thread) {
    return (
      <div className="flex h-full w-96 flex-col border-l border-border bg-bg-secondary">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <span className="text-sm font-semibold text-text-primary">Thread</span>
          <button onClick={handleClose} className="text-text-muted hover:text-text-primary">
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div className="flex flex-1 items-center justify-center text-sm text-text-muted">
          Thread not found
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full w-96 flex-col border-l border-border bg-bg-secondary">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold text-text-primary truncate">{thread.name}</span>
            {thread.archived && (
              <span className="rounded bg-bg-tertiary px-1.5 py-0.5 text-xs text-text-muted">Archived</span>
            )}
            {thread.channel_type === 'private_thread' && (
              <span className="rounded bg-bg-tertiary px-1.5 py-0.5 text-xs text-text-muted">Private</span>
            )}
          </div>
          <div className="text-xs text-text-muted">
            {thread.message_count} message{thread.message_count !== 1 ? 's' : ''}
          </div>
        </div>
        <div className="flex items-center gap-1">
          {!thread.archived && (
            <button
              onClick={handleArchive}
              className="rounded p-1 text-text-muted transition-colors hover:bg-bg-hover hover:text-text-primary"
              title="Archive thread"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4" />
              </svg>
            </button>
          )}
          {thread.archived && (
            <button
              onClick={handleUnarchive}
              className="rounded px-2 py-1 text-xs text-text-muted transition-colors hover:bg-bg-hover hover:text-text-primary"
              title="Unarchive thread"
            >
              Unarchive
            </button>
          )}
          <button onClick={handleClose} className="rounded p-1 text-text-muted transition-colors hover:bg-bg-hover hover:text-text-primary">
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>

      {forumTags.length > 0 && (
        <div className="flex flex-wrap gap-1 border-b border-border px-3 py-2" aria-label="Thread tags">
          {forumTags.map((tag) => {
            const selected = (thread.tag_ids ?? []).includes(tag.id);
            return (
              <button
                key={tag.id}
                type="button"
                aria-pressed={selected}
                onClick={() => toggleTag(tag.id)}
                className={`rounded-full border px-2 py-0.5 text-xs transition-colors ${selected
                  ? 'border-accent bg-accent/20 text-text-primary'
                  : 'border-border text-text-muted hover:border-accent hover:text-text-primary'}`}
                title={tag.moderated ? 'Moderator-controlled tag' : undefined}
              >
                {tag.emoji ? `${tag.emoji} ` : ''}{tag.name}{tag.moderated ? ' ◆' : ''}
              </button>
            );
          })}
        </div>
      )}

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-3 py-2">
        {messages.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-text-muted">
            No messages yet. Start the conversation!
          </div>
        ) : (
          messages.map((msg) => {
            const avatarUrl = avatars[msg.from];
            const time = new Date(msg.timestamp);
            const timeStr = time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
            return (
              <div key={msg.id} className="mb-2 flex gap-2">
                <div className="mt-0.5 shrink-0">
                  {avatarUrl ? (
                    <img src={avatarUrl} alt={msg.from} className="h-6 w-6 rounded-full object-cover" />
                  ) : (
                    <div className="flex h-6 w-6 items-center justify-center rounded-full bg-bg-accent text-xs font-bold text-white">
                      {msg.from[0]?.toUpperCase() || '?'}
                    </div>
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline gap-1">
                    <span className="text-sm font-medium text-text-primary">{msg.from}</span>
                    <span className="text-xs text-text-muted">{timeStr}</span>
                  </div>
                  <FormattedMessage content={msg.content} />
                </div>
              </div>
            );
          })
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      {!thread.archived && (
        <div className="border-t border-border p-3">
          <div className="flex gap-2">
            <input
              type="text"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Reply to thread..."
              className="flex-1 rounded border border-border bg-bg-primary px-3 py-1.5 text-sm text-text-primary outline-none placeholder:text-text-muted focus:border-accent"
            />
            <button
              onClick={handleSend}
              disabled={!input.trim()}
              className="rounded bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
            >
              Send
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
