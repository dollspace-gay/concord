import { useState, type KeyboardEvent } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';

export function MessageInput() {
  const [text, setText] = useState('');
  const activeChannel = useUiStore((s) => s.activeChannel);
  const sendMessage = useChatStore((s) => s.sendMessage);

  const handleSend = () => {
    const trimmed = text.trim();
    if (!trimmed || !activeChannel) return;
    sendMessage(activeChannel, trimmed);
    setText('');
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  if (!activeChannel) return null;

  return (
    <div className="px-4 pb-6 pt-1">
      <div className="flex items-center rounded-lg bg-bg-input px-4">
        <input
          type="text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={`Message ${activeChannel}`}
          className="flex-1 bg-transparent py-3 text-text-primary placeholder-text-muted outline-none"
        />
        <button
          onClick={handleSend}
          disabled={!text.trim()}
          className="ml-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary disabled:opacity-30"
        >
          <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
            <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z" />
          </svg>
        </button>
      </div>
    </div>
  );
}
