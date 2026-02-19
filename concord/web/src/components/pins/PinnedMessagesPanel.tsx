import { useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import type { PinnedMessageInfo } from '../../api/types';

const EMPTY_PINS: PinnedMessageInfo[] = [];

export function PinnedMessagesPanel() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const setShowPinnedMessages = useUiStore((s) => s.setShowPinnedMessages);
  const key = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const pins = useChatStore((s) => (key ? s.pinnedMessages[key] ?? EMPTY_PINS : EMPTY_PINS));
  const getPinnedMessages = useChatStore((s) => s.getPinnedMessages);
  const unpinMessage = useChatStore((s) => s.unpinMessage);

  useEffect(() => {
    if (activeServer && activeChannel) {
      getPinnedMessages(activeServer, activeChannel);
    }
  }, [activeServer, activeChannel, getPinnedMessages]);

  const handleJumpToMessage = () => {
    // Future: scroll to specific message ID within the current channel
  };

  const handleUnpin = (pin: PinnedMessageInfo) => {
    if (activeServer && activeChannel) {
      unpinMessage(activeServer, activeChannel, pin.message_id);
    }
  };

  return (
    <div className="flex h-full w-80 flex-col border-l border-border bg-bg-secondary">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-semibold text-text-primary">Pinned Messages</span>
        <button
          onClick={() => setShowPinnedMessages(false)}
          className="text-text-muted hover:text-text-primary"
        >
          <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {pins.length === 0 ? (
          <div className="flex flex-col items-center justify-center p-6 text-center text-text-muted">
            <svg className="mb-2 h-10 w-10" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25v13.5m-7.5-13.5v13.5" />
            </svg>
            <p className="text-sm">No pinned messages in this channel.</p>
            <p className="mt-1 text-xs">Pin important messages so they are easy to find.</p>
          </div>
        ) : (
          <div className="px-1 py-1">
            {pins.map((pin) => (
              <div
                key={pin.id}
                className="mx-1 mb-1 rounded border border-border bg-bg-primary p-3"
              >
                <div className="flex items-baseline gap-2">
                  <span className="text-sm font-medium text-text-primary">{pin.from}</span>
                  <span className="text-xs text-text-muted">
                    {new Date(pin.timestamp).toLocaleDateString()}
                  </span>
                </div>
                <div className="mt-1 text-sm text-text-secondary line-clamp-3">
                  {pin.content}
                </div>
                <div className="mt-1 text-xs text-text-muted">
                  Pinned by {pin.pinned_by} on {new Date(pin.pinned_at).toLocaleDateString()}
                </div>
                <div className="mt-2 flex gap-2">
                  <button
                    onClick={() => handleJumpToMessage()}
                    className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-secondary transition-colors hover:bg-bg-hover hover:text-text-primary"
                  >
                    Jump to message
                  </button>
                  <button
                    onClick={() => handleUnpin(pin)}
                    className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-secondary transition-colors hover:bg-bg-hover hover:text-red-400"
                  >
                    Unpin
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
