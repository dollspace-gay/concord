import { useCallback, useEffect, useRef, useState } from 'react';
import { Virtuoso, type VirtuosoHandle } from 'react-virtuoso';
import * as api from '../../api/client';
import { channelKey } from '../../api/types';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { EMPTY_MESSAGES, EMPTY_TYPING } from './messages/defaults';
import { MessageItem } from './messages/item';

export function MessageList() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const activeDirectConversation = useUiStore((s) => s.activeDirectConversation);
  const directConversation = useChatStore((s) => s.directConversations.find((dm) => dm.id === activeDirectConversation));
  const key = activeDirectConversation ? `dm:${activeDirectConversation}` : activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const messages = useChatStore((s) => (key ? s.messages[key] ?? EMPTY_MESSAGES : EMPTY_MESSAGES));
  const hasMore = useChatStore((s) => (key ? s.hasMore[key] ?? true : false));
  const typingUsers = useChatStore((s) => (key ? s.typingUsers[key] ?? EMPTY_TYPING : EMPTY_TYPING));
  const fetchHistory = useChatStore((s) => s.fetchHistory);
  const markDirectRead = useChatStore((s) => s.markDirectRead);
  const loadServerEmoji = useChatStore((s) => s.loadServerEmoji);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const jumpToMessageId = useUiStore((s) => s.jumpToMessageId);
  const setJumpToMessageId = useUiStore((s) => s.setJumpToMessageId);
  const prevLengthRef = useRef(0);
  const isFetchingRef = useRef(false);
  const lastDirectReadRef = useRef<string | null>(null);
  const [publicationPolicy, setPublicationPolicy] = useState<{ channelId: string; allowed: boolean } | null>(null);
  const publicationAllowed = publicationPolicy?.channelId === activeChannel && publicationPolicy.allowed;

  useEffect(() => {
    let current = true;
    if (!activeChannel || activeDirectConversation) return () => { current = false; };
    void api.getAtprotoChannelPublicationPolicy(activeChannel).then((policy) => {
      if (current) setPublicationPolicy({ channelId: activeChannel, allowed: policy.eligible && policy.channel_enabled && policy.user_granted });
    }).catch(() => { if (current) setPublicationPolicy({ channelId: activeChannel, allowed: false }); });
    return () => { current = false; };
  }, [activeChannel, activeDirectConversation]);

  // Load custom emoji when active server changes
  useEffect(() => {
    if (activeServer) {
      loadServerEmoji(activeServer);
    }
  }, [activeServer, loadServerEmoji]);

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
    // Reset fetch guard when messages change (history response arrived)
    isFetchingRef.current = false;
  }, [messages.length]);

  useEffect(() => {
    if (!jumpToMessageId) return;
    const index = messages.findIndex((message) => message.id === jumpToMessageId);
    if (index < 0) return;
    virtuosoRef.current?.scrollToIndex({ index, align: 'center', behavior: 'smooth' });
    setJumpToMessageId(null);
    const highlightTimer = window.setTimeout(() => {
      const element = document.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(jumpToMessageId)}"]`);
      element?.classList.add('bg-accent/20', 'ring-1', 'ring-inset', 'ring-accent');
      if (element) window.setTimeout(() => element.classList.remove('bg-accent/20', 'ring-1', 'ring-inset', 'ring-accent'), 2_500);
    }, 50);
    return () => window.clearTimeout(highlightTimer);
  }, [jumpToMessageId, messages, setJumpToMessageId]);

  useEffect(() => {
    if (!activeDirectConversation || messages.length === 0) return;
    const markVisible = () => {
      if (document.visibilityState !== 'visible') return;
      const lastMessage = messages[messages.length - 1];
      if (!lastMessage.sequence) return;
      const readKey = `${activeDirectConversation}:${lastMessage.id}`;
      if (lastDirectReadRef.current === readKey) return;
      lastDirectReadRef.current = readKey;
      markDirectRead(activeDirectConversation, lastMessage.id);
    };
    markVisible();
    document.addEventListener('visibilitychange', markVisible);
    return () => document.removeEventListener('visibilitychange', markVisible);
  }, [activeDirectConversation, messages, markDirectRead]);

  const handleLoadMore = useCallback(() => {
    if (isFetchingRef.current || !activeServer || !activeChannel || !hasMore || messages.length === 0) return;
    const oldest = messages[0];
    if (oldest) {
      isFetchingRef.current = true;
      fetchHistory(activeServer, activeChannel, oldest.id);
    }
  }, [activeServer, activeChannel, hasMore, messages, fetchHistory]);

  if (!activeChannel && !activeDirectConversation) {
    return (
      <div className="flex flex-1 items-center justify-center text-text-muted">
        Select a channel to start chatting
      </div>
    );
  }

  if (messages.length === 0) {
    return (
      <div className="flex flex-1 flex-col">
        <div className="flex flex-1 items-center justify-center text-text-muted">
          <div className="text-center">
            <p className="mb-1 text-2xl font-bold text-text-primary">
              {activeDirectConversation ? `@${directConversation?.peer_username ?? 'Direct message'}` : `Welcome to ${activeChannel}`}
            </p>
            <p>This is the beginning of the channel.</p>
          </div>
        </div>
        <TypingIndicator users={typingUsers} />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col">
      <Virtuoso
        key={key}
        ref={virtuosoRef}
        data={messages}
        initialTopMostItemIndex={{ index: 'LAST', align: 'end' }}
        defaultItemHeight={80}
        startReached={handleLoadMore}
        followOutput="smooth"
        className="flex-1"
        itemContent={(_index, msg) => <MessageItem message={msg} publicationAllowed={publicationAllowed} />}
      />
      <TypingIndicator users={typingUsers} />
    </div>
  );
}

function TypingIndicator({ users }: { users: string[] }) {
  if (users.length === 0) return null;

  const text =
    users.length === 1
      ? `${users[0]} is typing...`
      : users.length === 2
        ? `${users[0]} and ${users[1]} are typing...`
        : `${users[0]} and ${users.length - 1} others are typing...`;

  return (
    <div className="px-4 pb-1 text-xs text-text-muted">
      {text}
    </div>
  );
}
