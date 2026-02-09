import { useEffect, useRef } from 'react';
import { Virtuoso, type VirtuosoHandle } from 'react-virtuoso';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import type { HistoryMessage } from '../../api/types';

const EMPTY_MESSAGES: HistoryMessage[] = [];

export function MessageList() {
  const activeChannel = useUiStore((s) => s.activeChannel);
  const messages = useChatStore((s) => (activeChannel ? s.messages[activeChannel] ?? EMPTY_MESSAGES : EMPTY_MESSAGES));
  const hasMore = useChatStore((s) => (activeChannel ? s.hasMore[activeChannel] ?? true : false));
  const fetchHistory = useChatStore((s) => s.fetchHistory);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const prevLengthRef = useRef(0);

  // Auto-scroll to bottom when new messages arrive at the end
  useEffect(() => {
    if (messages.length > prevLengthRef.current) {
      // Only auto-scroll if the new message was appended (not prepended via history)
      const wasAppend = messages.length - prevLengthRef.current < 5;
      if (wasAppend && prevLengthRef.current > 0) {
        virtuosoRef.current?.scrollToIndex({ index: messages.length - 1, behavior: 'smooth' });
      }
    }
    prevLengthRef.current = messages.length;
  }, [messages.length]);

  const handleLoadMore = () => {
    if (!activeChannel || !hasMore || messages.length === 0) return;
    const oldest = messages[0];
    if (oldest) {
      fetchHistory(activeChannel, oldest.id);
    }
  };

  if (!activeChannel) {
    return (
      <div className="flex flex-1 items-center justify-center text-text-muted">
        Select a channel to start chatting
      </div>
    );
  }

  if (messages.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center text-text-muted">
        <div className="text-center">
          <p className="mb-1 text-2xl font-bold text-text-primary">
            Welcome to {activeChannel}
          </p>
          <p>This is the beginning of the channel.</p>
        </div>
      </div>
    );
  }

  return (
    <Virtuoso
      ref={virtuosoRef}
      data={messages}
      startReached={handleLoadMore}
      followOutput="smooth"
      className="flex-1"
      itemContent={(_index, msg) => <MessageItem message={msg} />}
    />
  );
}

function MessageItem({ message }: { message: HistoryMessage }) {
  const time = new Date(message.timestamp);
  const timeStr = time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

  return (
    <div className="group flex gap-4 px-4 py-1 hover:bg-bg-hover">
      <div className="mt-1 flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-bg-accent text-sm font-bold text-white">
        {message.from[0]?.toUpperCase() || '?'}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="font-medium text-text-primary">{message.from}</span>
          <span className="text-xs text-text-muted">{timeStr}</span>
        </div>
        <p className="whitespace-pre-wrap break-words text-text-secondary">{message.content}</p>
      </div>
    </div>
  );
}
