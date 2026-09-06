import { useState } from 'react';
import type { HistoryMessage } from '../../../api/types';
import { channelKey } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';
import { useComposerStore } from '../../../stores/composerStore';
import { useUiStore } from '../../../stores/uiStore';
import { Dialog } from '../../Dialog';
import { FormattedMessage } from '../FormattedMessage';
import { AttachmentPreview } from './attachments';
import { MessageComponents } from './components';
import { LinkEmbed, RichEmbed } from './embeds';
import { ReactionBadge } from './reactions';

export function MessageItem({ message, publicationAllowed }: { message: HistoryMessage; publicationAllowed: boolean }) {
  const avatarUrl = useChatStore((s) => s.avatars[message.from]);
  const nickname = useChatStore((s) => s.nickname);
  const activeAccountId = useChatStore((s) => s.activeAccountId);
  const editMessage = useChatStore((s) => s.editMessage);
  const deleteMessage = useChatStore((s) => s.deleteMessage);
  const addReaction = useChatStore((s) => s.addReaction);
  const setReplyFor = useComposerStore((s) => s.setReplyFor);
  const pinMessage = useChatStore((s) => s.pinMessage);
  const addBookmark = useChatStore((s) => s.addBookmark);
  const createThread = useChatStore((s) => s.createThread);
  const shareToBlueskyAction = useChatStore((s) => s.shareToBluesky);
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const activeDirectConversation = useUiStore((s) => s.activeDirectConversation);
  const time = new Date(message.timestamp);
  const timeStr = time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState(message.content);
  const [showActions, setShowActions] = useState(false);
  const [showThreadForm, setShowThreadForm] = useState(false);
  const [threadName, setThreadName] = useState(`thread-${message.id.slice(0, 8)}`);
  const [privateThread, setPrivateThread] = useState(false);

  const isOwn = message.sender_id ? message.sender_id === activeAccountId : message.from === nickname;

  const memberSenderId = useChatStore((state) => {
    if (!activeServer || !activeChannel) return undefined;
    return state.members[channelKey(activeServer, activeChannel)]
      ?.find((member) => member.nickname === message.from)?.user_id ?? undefined;
  });
  const senderId = message.sender_id ?? memberSenderId;
  const handleNameClick = () => {
    if (senderId) useUiStore.getState().setShowUserProfile(senderId);
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
    const key = activeDirectConversation
      ? `dm:${activeDirectConversation}`
      : activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
    if (!key) return;
    setReplyFor(key, {
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
    setShowThreadForm(true);
    setShowActions(false);
  };

  const submitThread = (event: React.FormEvent) => {
    event.preventDefault();
    const name = threadName.trim();
    if (activeServer && activeChannel) {
      if (!name) return;
      createThread(activeServer, activeChannel, name, message.id, privateThread);
      setShowThreadForm(false);
    }
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
    <>
      <div
        tabIndex={0}
        data-message-id={message.id}
        aria-label={`Message from ${message.from}`}
        className="group relative flex gap-4 px-4 py-1 transition-colors hover:bg-bg-hover"
        onMouseEnter={() => setShowActions(true)}
        onMouseLeave={() => setShowActions(false)}
        onFocus={() => setShowActions(true)}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) setShowActions(false);
        }}
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
                <LinkEmbed key={embed.url} embed={embed} messageId={message.id} />
              ))}
            </div>
          )}
          {message.rich_embeds && message.rich_embeds.length > 0 && (
            <div className="mt-2 flex max-w-[520px] flex-col gap-2">
              {message.rich_embeds.map((embed, index) => (
                <RichEmbed key={`${message.id}-embed-${index}`} embed={embed} messageId={message.id} />
              ))}
            </div>
          )}
          {message.components && message.components.length > 0 && (
            <MessageComponents messageId={message.id} components={message.components} />
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
            {isOwn && publicationAllowed && <button
              onClick={handleShareToBluesky}
              className={`px-1.5 py-0.5 text-sm hover:bg-bg-hover ${shareStatus === 'shared' ? 'text-green-400' : shareStatus === 'error' ? 'text-red-400' : 'text-text-muted hover:text-blue-400'}`}
              title={shareStatus === 'shared' ? 'Shared!' : shareStatus === 'error' ? 'Failed to share' : shareStatus === 'sharing' ? 'Sharing...' : 'Share to Bluesky'}
              disabled={shareStatus === 'sharing'}
            >
              <svg className="h-4 w-4" viewBox="0 0 600 530" fill="currentColor">
                <path d="m135.72 44.03c66.496 49.921 138.02 151.14 164.28 205.46 26.262-54.316 97.782-155.54 164.28-205.46 47.98-36.021 125.72-63.892 125.72 24.795 0 17.712-10.155 148.79-16.111 170.07-20.703 73.984-96.144 92.854-163.25 81.433 117.3 19.964 147.14 86.092 82.697 152.22-122.39 125.59-175.91-31.511-189.63-71.766-2.514-7.3797-3.6904-10.832-3.7077-7.8964-0.0174-2.9357-1.1937 0.51669-3.7077 7.8964-13.72 40.255-67.233 197.36-189.63 71.766-64.444-66.128-34.605-132.26 82.697-152.22-67.108 11.421-142.55-7.4491-163.25-81.433-5.9562-21.282-16.111-152.36-16.111-170.07 0-88.687 77.742-60.816 125.72-24.795z" />
              </svg>
            </button>}
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
      </div>
      {showThreadForm && (
        <Dialog label="Create thread" onClose={() => setShowThreadForm(false)}>
          <form onSubmit={submitThread} className="w-full max-w-sm rounded-lg bg-bg-secondary p-5 shadow-xl">
            <h2 className="mb-4 text-lg font-semibold text-text-primary">Create thread</h2>
            <label className="mb-4 block text-sm text-text-muted">
              Thread name
              <input
                autoFocus
                value={threadName}
                onChange={(event) => setThreadName(event.target.value)}
                maxLength={100}
                className="mt-1 w-full rounded border border-border bg-bg-primary px-3 py-2 text-text-primary outline-none focus:border-accent"
              />
            </label>
            <label className="mb-5 flex items-center gap-2 text-sm text-text-primary">
              <input
                type="checkbox"
                checked={privateThread}
                onChange={(event) => setPrivateThread(event.target.checked)}
              />
              Private thread
            </label>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setShowThreadForm(false)} className="rounded px-3 py-2 text-sm text-text-muted hover:bg-bg-hover">
                Cancel
              </button>
              <button type="submit" disabled={!threadName.trim()} className="rounded bg-accent px-3 py-2 text-sm font-medium text-white disabled:opacity-50">
                Create thread
              </button>
            </div>
          </form>
        </Dialog>
      )}
    </>
  );
}
