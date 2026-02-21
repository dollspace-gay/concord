import { useState, useRef, useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';

interface EmojiPickerProps {
  onSelect: (emoji: string) => void;
  onClose: () => void;
  serverId?: string | null;
}

const EMOJI_CATEGORIES: { name: string; emojis: string[] }[] = [
  {
    name: 'Smileys',
    emojis: [
      '😀', '😃', '😄', '😁', '😆', '😅', '🤣', '😂', '🙂', '😊',
      '😇', '🥰', '😍', '🤩', '😘', '😗', '😚', '😙', '🥲', '😋',
      '😛', '😜', '🤪', '😝', '🤑', '🤗', '🤭', '🤫', '🤔', '🫡',
      '🤐', '🤨', '😐', '😑', '😶', '🫥', '😏', '😒', '🙄', '😬',
      '😮‍💨', '🤥', '😌', '😔', '😪', '🤤', '😴', '😷', '🤒', '🤕',
      '🤢', '🤮', '🥴', '😵', '🤯', '🥳', '🥸', '😎', '🤓', '🧐',
      '😕', '🫤', '😟', '🙁', '😮', '😯', '😲', '😳', '🥺', '🥹',
      '😦', '😧', '😨', '😰', '😥', '😢', '😭', '😱', '😖', '😣',
      '😞', '😓', '😩', '😫', '🥱', '😤', '😡', '😠', '🤬', '😈',
      '👿', '💀', '☠️', '💩', '🤡', '👹', '👺', '👻', '👽', '🤖',
    ],
  },
  {
    name: 'Gestures',
    emojis: [
      '👋', '🤚', '🖐️', '✋', '🖖', '🫱', '🫲', '🫳', '🫴', '🫷',
      '🫸', '👌', '🤌', '🤏', '✌️', '🤞', '🫰', '🤟', '🤘', '🤙',
      '👈', '👉', '👆', '🖕', '👇', '☝️', '🫵', '👍', '👎', '✊',
      '👊', '🤛', '🤜', '👏', '🙌', '🫶', '👐', '🤲', '🤝', '🙏',
      '✍️', '💅', '🤳', '💪', '🦾', '🦿', '🦵', '🦶', '👂', '🦻',
    ],
  },
  {
    name: 'Hearts',
    emojis: [
      '❤️', '🧡', '💛', '💚', '💙', '💜', '🖤', '🤍', '🤎', '💔',
      '❤️‍🔥', '❤️‍🩹', '❣️', '💕', '💞', '💓', '💗', '💖', '💘', '💝',
      '💟', '♥️', '💋', '💯', '🔥', '✨', '⭐', '🌟', '💫', '💥',
    ],
  },
  {
    name: 'Objects',
    emojis: [
      '🎉', '🎊', '🎈', '🎁', '🎗️', '🏆', '🥇', '🥈', '🥉', '⚽',
      '🏀', '🏈', '⚾', '🎾', '🎮', '🕹️', '🎲', '🎯', '🎵', '🎶',
      '🎤', '🎧', '📱', '💻', '⌨️', '🖥️', '📷', '📹', '🔒', '🔑',
      '🔨', '🪓', '⚔️', '💣', '🪄', '💊', '🧪', '🔬', '🔭', '📡',
    ],
  },
  {
    name: 'Food',
    emojis: [
      '🍕', '🍔', '🍟', '🌭', '🍿', '🧂', '🥓', '🥚', '🍳', '🧇',
      '🥞', '🧈', '🍞', '🥐', '🥨', '🧀', '🥩', '🍗', '🍖', '🌮',
      '🌯', '🥙', '🍝', '🍜', '🍲', '🍛', '🍣', '🍱', '🥟', '🍤',
      '🍩', '🍪', '🎂', '🍰', '🧁', '🍫', '🍬', '🍭', '🍮', '🍯',
      '🍺', '🍻', '🥂', '🍷', '🍸', '🍹', '☕', '🍵', '🧃', '🥤',
    ],
  },
  {
    name: 'Nature',
    emojis: [
      '🐶', '🐱', '🐭', '🐹', '🐰', '🦊', '🐻', '🐼', '🐻‍❄️', '🐨',
      '🐯', '🦁', '🐮', '🐷', '🐸', '🐵', '🙈', '🙉', '🙊', '🐔',
      '🐧', '🐦', '🦅', '🦆', '🦉', '🐺', '🐗', '🐴', '🦄', '🐝',
      '🌸', '🌹', '🌺', '🌻', '🌼', '🌷', '🌱', '🌲', '🌳', '🍀',
      '🍁', '🍂', '🍃', '🌍', '🌎', '🌏', '🌑', '🌕', '⭐', '🌈',
    ],
  },
];

const EMPTY_EMOJI: Record<string, { id: string; image_url: string }> = {};

export function EmojiPicker({ onSelect, onClose, serverId }: EmojiPickerProps) {
  const customEmoji = useChatStore((s) => (serverId ? s.customEmoji[serverId] ?? EMPTY_EMOJI : EMPTY_EMOJI));
  const customEntries = Object.entries(customEmoji);
  const hasCustom = customEntries.length > 0;

  // -1 = server tab, 0+ = unicode categories
  const [activeCategory, setActiveCategory] = useState(hasCustom ? -1 : 0);
  const [search, setSearch] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const pickerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Close on click outside
  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [onClose]);

  const searchLower = search.toLowerCase();

  // Filter custom emoji by search
  const filteredCustom = search
    ? customEntries.filter(([name]) => name.toLowerCase().includes(searchLower))
    : customEntries;

  const allUnicode = EMOJI_CATEGORIES.flatMap((c) => c.emojis);
  const filteredUnicode = search
    ? allUnicode // Show all unicode when searching (no name metadata to filter)
    : activeCategory >= 0
      ? EMOJI_CATEGORIES[activeCategory].emojis
      : [];

  return (
    <div
      ref={pickerRef}
      className="absolute bottom-full right-0 mb-1 flex w-80 flex-col overflow-hidden rounded-lg border border-border bg-bg-secondary shadow-lg"
    >
      {/* Search */}
      <div className="border-b border-border px-3 py-2">
        <input
          ref={inputRef}
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search emoji..."
          className="w-full bg-transparent text-sm text-text-primary placeholder-text-muted outline-none"
          onKeyDown={(e) => {
            if (e.key === 'Escape') onClose();
          }}
        />
      </div>

      {/* Category tabs */}
      {!search && (
        <div className="flex border-b border-border px-1">
          {hasCustom && (
            <button
              onClick={() => setActiveCategory(-1)}
              className={`px-2 py-1.5 text-center text-xs transition-colors ${
                activeCategory === -1
                  ? 'border-b-2 border-blue-400 text-text-primary'
                  : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Server Emoji"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 12h14M12 5l7 7-7 7" />
              </svg>
            </button>
          )}
          {EMOJI_CATEGORIES.map((cat, i) => (
            <button
              key={cat.name}
              onClick={() => setActiveCategory(i)}
              className={`flex-1 px-1 py-1.5 text-center text-xs transition-colors ${
                i === activeCategory
                  ? 'border-b-2 border-blue-400 text-text-primary'
                  : 'text-text-muted hover:text-text-secondary'
              }`}
              title={cat.name}
            >
              {cat.emojis[0]}
            </button>
          ))}
        </div>
      )}

      {/* Emoji grid */}
      <div className="h-52 overflow-y-auto p-2">
        {/* Custom emoji section */}
        {(activeCategory === -1 || search) && filteredCustom.length > 0 && (
          <>
            {search && <p className="mb-1 text-xs font-semibold text-text-muted">Server Emoji</p>}
            <div className="grid grid-cols-8 gap-0.5">
              {filteredCustom.map(([name, emoji]) => (
                <button
                  key={name}
                  onClick={() => {
                    onSelect(`:${name}:`);
                    onClose();
                  }}
                  className="flex h-8 w-8 items-center justify-center rounded transition-colors hover:bg-bg-hover"
                  title={`:${name}:`}
                >
                  <img src={emoji.image_url} alt={name} className="h-6 w-6 object-contain" />
                </button>
              ))}
            </div>
          </>
        )}

        {/* Unicode emoji section */}
        {(activeCategory >= 0 || search) && (
          <>
            {search && filteredCustom.length > 0 && <p className="mb-1 mt-2 text-xs font-semibold text-text-muted">Unicode</p>}
            <div className="grid grid-cols-8 gap-0.5">
              {filteredUnicode.map((emoji, i) => (
                <button
                  key={`${emoji}-${i}`}
                  onClick={() => {
                    onSelect(emoji);
                    onClose();
                  }}
                  className="flex h-8 w-8 items-center justify-center rounded text-xl transition-colors hover:bg-bg-hover"
                  title={emoji}
                >
                  {emoji}
                </button>
              ))}
            </div>
          </>
        )}

        {/* Empty state for custom */}
        {activeCategory === -1 && !search && filteredCustom.length === 0 && (
          <p className="py-8 text-center text-sm text-text-muted">No custom emoji for this server</p>
        )}
      </div>
    </div>
  );
}
