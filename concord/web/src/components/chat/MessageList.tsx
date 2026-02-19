import { useEffect, useRef, useState, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { Virtuoso, type VirtuosoHandle } from 'react-virtuoso';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import { UserProfilePopup } from '../members/UserProfilePopup';
import { FormattedMessage } from './FormattedMessage';
import type { AttachmentInfo, EmbedInfo, HistoryMessage } from '../../api/types';
import { WaveformPlayer } from './WaveformPlayer';

const EMPTY_MESSAGES: HistoryMessage[] = [];
const EMPTY_TYPING: string[] = [];

export function MessageList() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const key = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const messages = useChatStore((s) => (key ? s.messages[key] ?? EMPTY_MESSAGES : EMPTY_MESSAGES));
  const hasMore = useChatStore((s) => (key ? s.hasMore[key] ?? true : false));
  const typingUsers = useChatStore((s) => (key ? s.typingUsers[key] ?? EMPTY_TYPING : EMPTY_TYPING));
  const fetchHistory = useChatStore((s) => s.fetchHistory);
  const loadServerEmoji = useChatStore((s) => s.loadServerEmoji);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const prevLengthRef = useRef(0);

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
  }, [messages.length]);

  const handleLoadMore = () => {
    if (!activeServer || !activeChannel || !hasMore || messages.length === 0) return;
    const oldest = messages[0];
    if (oldest) {
      fetchHistory(activeServer, activeChannel, oldest.id);
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
      <div className="flex flex-1 flex-col">
        <div className="flex flex-1 items-center justify-center text-text-muted">
          <div className="text-center">
            <p className="mb-1 text-2xl font-bold text-text-primary">
              Welcome to {activeChannel}
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
        ref={virtuosoRef}
        data={messages}
        startReached={handleLoadMore}
        followOutput="smooth"
        className="flex-1"
        itemContent={(_index, msg) => <MessageItem message={msg} />}
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

function MessageItem({ message }: { message: HistoryMessage }) {
  const avatars = useChatStore((s) => s.avatars);
  const nickname = useChatStore((s) => s.nickname);
  const editMessage = useChatStore((s) => s.editMessage);
  const deleteMessage = useChatStore((s) => s.deleteMessage);
  const addReaction = useChatStore((s) => s.addReaction);
  const setReplyingTo = useChatStore((s) => s.setReplyingTo);
  const pinMessage = useChatStore((s) => s.pinMessage);
  const addBookmark = useChatStore((s) => s.addBookmark);
  const createThread = useChatStore((s) => s.createThread);
  const shareToBlueskyAction = useChatStore((s) => s.shareToBluesky);
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const avatarUrl = avatars[message.from];
  const time = new Date(message.timestamp);
  const timeStr = time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const [showPopup, setShowPopup] = useState(false);
  const [popupPos, setPopupPos] = useState<{ top: number; left: number } | null>(null);
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState(message.content);
  const [showActions, setShowActions] = useState(false);

  const isOwn = message.from === nickname;

  const handleNameClick = (e: React.MouseEvent) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setPopupPos({ top: rect.bottom + 4, left: rect.left });
    setShowPopup(true);
  };

  const handleEditSubmit = () => {
    const trimmed = editText.trim();
    if (trimmed && trimmed !== message.content) {
      editMessage(message.id, trimmed);
    }
    setEditing(false);
  };

  const handleEditKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleEditSubmit();
    } else if (e.key === 'Escape') {
      setEditing(false);
      setEditText(message.content);
    }
  };

  const handleReply = () => {
    setReplyingTo({
      id: message.id,
      from: message.from,
      content_preview: message.content.slice(0, 100),
    });
  };

  const handleQuickReact = (emoji: string) => {
    addReaction(message.id, emoji);
    setShowActions(false);
  };

  const handlePin = () => {
    if (activeServer && activeChannel) {
      pinMessage(activeServer, activeChannel, message.id);
    }
    setShowActions(false);
  };

  const handleBookmark = () => {
    addBookmark(message.id);
    setShowActions(false);
  };

  const handleCreateThread = () => {
    if (activeServer && activeChannel) {
      const threadName = `thread-${message.id.slice(0, 8)}`;
      createThread(activeServer, activeChannel, threadName, message.id);
    }
    setShowActions(false);
  };

  const [shareStatus, setShareStatus] = useState<'idle' | 'sharing' | 'shared' | 'error'>('idle');
  const handleShareToBluesky = async () => {
    setShareStatus('sharing');
    try {
      await shareToBlueskyAction(message.id);
      setShareStatus('shared');
      setTimeout(() => setShareStatus('idle'), 2000);
    } catch (e) {
      console.error('Share to Bluesky failed:', e);
      setShareStatus('error');
      setTimeout(() => setShareStatus('idle'), 3000);
    }
  };

  return (
    <div
      className="group relative flex gap-4 px-4 py-1 hover:bg-bg-hover"
      onMouseEnter={() => setShowActions(true)}
      onMouseLeave={() => setShowActions(false)}
    >
      <button onClick={handleNameClick} className="mt-1 shrink-0">
        {avatarUrl ? (
          <img
            src={avatarUrl}
            alt={message.from}
            className="h-10 w-10 rounded-full object-cover"
          />
        ) : (
          <div className="flex h-10 w-10 items-center justify-center rounded-full bg-bg-accent text-sm font-bold text-white">
            {message.from[0]?.toUpperCase() || '?'}
          </div>
        )}
      </button>
      <div className="min-w-0 flex-1">
        {/* Reply preview */}
        {message.reply_to && (
          <div className="mb-1 flex items-center gap-1 text-xs text-text-muted">
            <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M3 10h10a5 5 0 015 5v3M3 10l4-4M3 10l4 4" />
            </svg>
            <span className="font-medium text-text-primary">{message.reply_to.from}</span>
            <span className="truncate">{message.reply_to.content_preview}</span>
          </div>
        )}
        <div className="flex items-baseline gap-2">
          <button
            onClick={handleNameClick}
            className="font-medium text-text-primary hover:underline"
          >
            {message.from}
          </button>
          <span className="text-xs text-text-muted">{timeStr}</span>
          {message.edited_at && (
            <span className="text-xs text-text-muted" title={`Edited ${new Date(message.edited_at).toLocaleString()}`}>
              (edited)
            </span>
          )}
        </div>
        {editing ? (
          <div className="mt-1">
            <input
              type="text"
              value={editText}
              onChange={(e) => setEditText(e.target.value)}
              onKeyDown={handleEditKeyDown}
              onBlur={handleEditSubmit}
              className="w-full rounded bg-bg-input px-2 py-1 text-text-primary outline-none"
              autoFocus
            />
            <div className="mt-1 text-xs text-text-muted">
              Enter to save, Escape to cancel
            </div>
          </div>
        ) : (
          <FormattedMessage content={message.content} />
        )}
        {/* Attachments */}
        {message.attachments && message.attachments.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-2">
            {message.attachments.map((att) => (
              <AttachmentPreview key={att.id} attachment={att} />
            ))}
          </div>
        )}
        {/* Link embeds */}
        {message.embeds && message.embeds.length > 0 && (
          <div className="mt-1 flex flex-col gap-2">
            {message.embeds.map((embed) => (
              <LinkEmbed key={embed.url} embed={embed} />
            ))}
          </div>
        )}
        {/* Reaction badges */}
        {message.reactions && message.reactions.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {message.reactions.map((r) => (
              <ReactionBadge
                key={r.emoji}
                emoji={r.emoji}
                count={r.count}
                messageId={message.id}
                userIds={r.user_ids}
              />
            ))}
          </div>
        )}
      </div>
      {/* Action buttons (visible on hover) */}
      {showActions && !editing && (
        <div className="absolute -top-3 right-4 flex gap-0.5 rounded border border-border bg-bg-secondary shadow-sm">
          <button
            onClick={() => handleQuickReact('👍')}
            className="px-1.5 py-0.5 text-sm hover:bg-bg-hover"
            title="React"
          >
            👍
          </button>
          <button
            onClick={handleReply}
            className="px-1.5 py-0.5 text-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
            title="Reply"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M3 10h10a5 5 0 015 5v3M3 10l4-4M3 10l4 4" />
            </svg>
          </button>
          <button
            onClick={handleCreateThread}
            className="px-1.5 py-0.5 text-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
            title="Create Thread"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M7 8h10M7 12h4m1 8l-4-4H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-3l-4 4z" />
            </svg>
          </button>
          <button
            onClick={handlePin}
            className="px-1.5 py-0.5 text-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
            title="Pin Message"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25v13.5m-7.5-13.5v13.5" />
            </svg>
          </button>
          <button
            onClick={handleShareToBluesky}
            className={`px-1.5 py-0.5 text-sm hover:bg-bg-hover ${shareStatus === 'shared' ? 'text-green-400' : shareStatus === 'error' ? 'text-red-400' : 'text-text-muted hover:text-blue-400'}`}
            title={shareStatus === 'shared' ? 'Shared!' : shareStatus === 'error' ? 'Failed to share' : shareStatus === 'sharing' ? 'Sharing...' : 'Share to Bluesky'}
            disabled={shareStatus === 'sharing'}
          >
            <svg className="h-4 w-4" viewBox="0 0 600 530" fill="currentColor">
              <path d="m135.72 44.03c66.496 49.921 138.02 151.14 164.28 205.46 26.262-54.316 97.782-155.54 164.28-205.46 47.98-36.021 125.72-63.892 125.72 24.795 0 17.712-10.155 148.79-16.111 170.07-20.703 73.984-96.144 92.854-163.25 81.433 117.3 19.964 147.14 86.092 82.697 152.22-122.39 125.59-175.91-31.511-189.63-71.766-2.514-7.3797-3.6904-10.832-3.7077-7.8964-0.0174-2.9357-1.1937 0.51669-3.7077 7.8964-13.72 40.255-67.233 197.36-189.63 71.766-64.444-66.128-34.605-132.26 82.697-152.22-67.108 11.421-142.55-7.4491-163.25-81.433-5.9562-21.282-16.111-152.36-16.111-170.07 0-88.687 77.742-60.816 125.72-24.795z" />
            </svg>
          </button>
          <button
            onClick={handleBookmark}
            className="px-1.5 py-0.5 text-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
            title="Bookmark"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 5a2 2 0 012-2h10a2 2 0 012 2v16l-7-3.5L5 21V5z" />
            </svg>
          </button>
          {isOwn && (
            <>
              <button
                onClick={() => { setEditing(true); setEditText(message.content); }}
                className="px-1.5 py-0.5 text-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
                title="Edit"
              >
                <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
                </svg>
              </button>
              <button
                onClick={() => deleteMessage(message.id)}
                className="px-1.5 py-0.5 text-sm text-text-muted hover:bg-bg-hover hover:text-red-400"
                title="Delete"
              >
                <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                </svg>
              </button>
            </>
          )}
        </div>
      )}
      {showPopup && popupPos && (
        <UserProfilePopup
          nickname={message.from}
          position={popupPos}
          onClose={() => setShowPopup(false)}
        />
      )}
    </div>
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function AttachmentPreview({ attachment }: { attachment: AttachmentInfo }) {
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const isImage = attachment.content_type.startsWith('image/');
  const isVideo = attachment.content_type.startsWith('video/');
  const isAudio = attachment.content_type.startsWith('audio/');

  if (isImage) {
    return (
      <>
        <button onClick={() => setLightboxOpen(true)} className="block cursor-zoom-in">
          <img
            src={attachment.url}
            alt={attachment.filename}
            className="max-h-[300px] max-w-[400px] rounded border border-border object-contain"
            loading="lazy"
          />
        </button>
        {lightboxOpen && (
          <ImageLightbox
            url={attachment.url}
            filename={attachment.filename}
            onClose={() => setLightboxOpen(false)}
          />
        )}
      </>
    );
  }

  if (isVideo) {
    return (
      <div className="max-w-[480px]">
        <video
          src={attachment.url}
          controls
          preload="metadata"
          className="max-h-[360px] w-full rounded border border-border"
        />
        <div className="mt-1 text-xs text-text-muted">{attachment.filename} — {formatFileSize(attachment.file_size)}</div>
      </div>
    );
  }

  if (isAudio) {
    return (
      <WaveformPlayer
        src={attachment.url}
        filename={attachment.filename}
        fileSize={attachment.file_size}
      />
    );
  }

  return (
    <a
      href={attachment.url}
      target="_blank"
      rel="noopener noreferrer"
      className="flex items-center gap-2 rounded border border-border bg-bg-secondary px-3 py-2 text-sm transition-colors hover:bg-bg-hover"
    >
      <svg className="h-5 w-5 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" />
      </svg>
      <div className="min-w-0">
        <div className="truncate font-medium text-text-primary">{attachment.filename}</div>
        <div className="text-xs text-text-muted">{formatFileSize(attachment.file_size)}</div>
      </div>
      <svg className="h-4 w-4 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
      </svg>
    </a>
  );
}

function ImageLightbox({ url, filename, onClose }: { url: string; filename: string; onClose: () => void }) {
  const [scale, setScale] = useState(1);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const dragging = useRef(false);
  const lastPos = useRef({ x: 0, y: 0 });

  const handleKeyDown = useCallback((e: globalThis.KeyboardEvent) => {
    if (e.key === 'Escape') onClose();
  }, [onClose]);

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [handleKeyDown]);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    setScale((s) => Math.min(Math.max(0.25, s - e.deltaY * 0.001), 10));
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0) return;
    dragging.current = true;
    setIsDragging(true);
    lastPos.current = { x: e.clientX, y: e.clientY };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!dragging.current) return;
    const dx = e.clientX - lastPos.current.x;
    const dy = e.clientY - lastPos.current.y;
    lastPos.current = { x: e.clientX, y: e.clientY };
    setTranslate((t) => ({ x: t.x + dx, y: t.y + dy }));
  }, []);

  const handlePointerUp = useCallback(() => {
    dragging.current = false;
    setIsDragging(false);
  }, []);

  const resetView = useCallback(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, []);

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      {/* Top bar */}
      <div className="absolute top-0 left-0 right-0 flex items-center justify-between px-4 py-3 text-white">
        <span className="truncate text-sm font-medium">{filename}</span>
        <div className="flex items-center gap-2">
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded p-1.5 hover:bg-white/10"
            title="Open original"
          >
            <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
            </svg>
          </a>
          <button onClick={onClose} className="rounded p-1.5 hover:bg-white/10" title="Close">
            <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>
      {/* Zoom controls */}
      <div className="absolute bottom-4 left-1/2 flex -translate-x-1/2 items-center gap-1 rounded-lg bg-black/60 px-2 py-1 text-white">
        <button onClick={() => setScale((s) => Math.max(0.25, s / 1.5))} className="px-2 py-1 hover:bg-white/10 rounded" title="Zoom out">−</button>
        <button onClick={resetView} className="px-2 py-1 text-xs hover:bg-white/10 rounded" title="Reset zoom">{Math.round(scale * 100)}%</button>
        <button onClick={() => setScale((s) => Math.min(10, s * 1.5))} className="px-2 py-1 hover:bg-white/10 rounded" title="Zoom in">+</button>
      </div>
      {/* Image */}
      <img
        src={url}
        alt={filename}
        className="max-h-[90vh] max-w-[90vw] select-none"
        style={{
          transform: `translate(${translate.x}px, ${translate.y}px) scale(${scale})`,
          cursor: isDragging ? 'grabbing' : 'grab',
        }}
        draggable={false}
        onWheel={handleWheel}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
      />
    </div>,
    document.body,
  );
}

function LinkEmbed({ embed }: { embed: EmbedInfo }) {
  return (
    <a
      href={embed.url}
      target="_blank"
      rel="noopener noreferrer"
      className="flex max-w-[480px] overflow-hidden rounded border-l-4 border-blue-500 bg-bg-secondary transition-colors hover:bg-bg-hover"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1 p-3">
        {embed.site_name && (
          <span className="text-xs text-text-muted">{embed.site_name}</span>
        )}
        {embed.title && (
          <span className="text-sm font-semibold text-blue-400">{embed.title}</span>
        )}
        {embed.description && (
          <span className="line-clamp-3 text-sm text-text-secondary">{embed.description}</span>
        )}
      </div>
      {embed.image_url && (
        <img
          src={embed.image_url}
          alt=""
          className="h-20 w-20 shrink-0 object-cover"
          loading="lazy"
        />
      )}
    </a>
  );
}

function ReactionBadge({
  emoji,
  count,
  messageId,
  userIds,
}: {
  emoji: string;
  count: number;
  messageId: string;
  userIds: string[];
}) {
  const nickname = useChatStore((s) => s.nickname);
  const addReaction = useChatStore((s) => s.addReaction);
  const removeReaction = useChatStore((s) => s.removeReaction);

  // Check if current user has reacted (by nickname since we may not have user_id client-side)
  const hasReacted = userIds.includes(nickname || '');

  const handleClick = () => {
    if (hasReacted) {
      removeReaction(messageId, emoji);
    } else {
      addReaction(messageId, emoji);
    }
  };

  return (
    <button
      onClick={handleClick}
      className={`flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs transition-colors ${
        hasReacted
          ? 'border-blue-500/50 bg-blue-500/10 text-text-primary'
          : 'border-border bg-bg-secondary text-text-muted hover:bg-bg-hover'
      }`}
    >
      <span>{emoji}</span>
      <span>{count}</span>
    </button>
  );
}
