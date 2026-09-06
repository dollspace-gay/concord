import { useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import type { BookmarkInfo, ChannelInfo } from '../../api/types';

const EMPTY_BOOKMARKS: BookmarkInfo[] = [];
const EMPTY_CHANNELS: ChannelInfo[] = [];

export function BookmarksPanel() {
  const setShowBookmarks = useUiStore((s) => s.setShowBookmarks);
  const activeServer = useUiStore((s) => s.activeServer);
  const channels = useChatStore((s) => activeServer ? s.channels[activeServer] ?? EMPTY_CHANNELS : EMPTY_CHANNELS);
  const setActiveChannel = useUiStore((s) => s.setActiveChannel);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const bookmarks = useChatStore((s) => s.bookmarks ?? EMPTY_BOOKMARKS);
  const listBookmarks = useChatStore((s) => s.listBookmarks);
  const removeBookmark = useChatStore((s) => s.removeBookmark);
  const setJumpToMessageId = useUiStore((s) => s.setJumpToMessageId);

  useEffect(() => {
    listBookmarks();
  }, [listBookmarks]);

  const handleJumpToMessage = (bookmark: BookmarkInfo) => {
    // Navigate to the channel containing the bookmarked message
    if (activeServer && bookmark.channel_id) {
      const channel = channels.find((candidate) => candidate.id === bookmark.channel_id);
      if (!channel) return;
      setActiveChannel(channel.name);
      joinChannel(activeServer, channel.name);
    }
    setJumpToMessageId(bookmark.message_id);
    setShowBookmarks(false);
  };

  return (
    <div className="flex h-full w-80 flex-col border-l border-border bg-bg-secondary">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-semibold text-text-primary">Bookmarks</span>
        <button
          onClick={() => setShowBookmarks(false)}
          className="text-text-muted hover:text-text-primary"
          aria-label="Close bookmarks"
        >
          <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {bookmarks.length === 0 ? (
          <div className="flex flex-col items-center justify-center p-6 text-center text-text-muted">
            <svg className="mb-2 h-10 w-10" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 5a2 2 0 012-2h10a2 2 0 012 2v16l-7-3.5L5 21V5z" />
            </svg>
            <p className="text-sm">No bookmarks yet.</p>
            <p className="mt-1 text-xs">Bookmark messages to save them for later.</p>
          </div>
        ) : (
          <div className="px-1 py-1">
            {bookmarks.map((bookmark) => (
              <div
                key={bookmark.id}
                className="mx-1 mb-1 rounded border border-border bg-bg-primary p-3"
              >
                <div className="flex items-baseline gap-2">
                  <span className="text-sm font-medium text-text-primary">{bookmark.from}</span>
                  <span className="text-xs text-text-muted">
                    {new Date(bookmark.timestamp).toLocaleDateString()}
                  </span>
                </div>
                <div className="mt-1 text-sm text-text-secondary line-clamp-3">
                  {bookmark.content}
                </div>
                {bookmark.note && (
                  <div className="mt-1 rounded bg-bg-tertiary px-2 py-1 text-xs text-text-muted italic">
                    Note: {bookmark.note}
                  </div>
                )}
                <div className="mt-1 text-xs text-text-muted">
                  Saved on {new Date(bookmark.created_at).toLocaleDateString()}
                </div>
                <div className="mt-2 flex gap-2">
                  <button
                    onClick={() => handleJumpToMessage(bookmark)}
                    className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-secondary transition-colors hover:bg-bg-hover hover:text-text-primary"
                  >
                    Jump to message
                  </button>
                  <button
                    onClick={() => removeBookmark(bookmark.message_id)}
                    className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-secondary transition-colors hover:bg-bg-hover hover:text-red-400"
                  >
                    Remove
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
