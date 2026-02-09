import { useState, useRef, useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';

interface Props {
  onClose: () => void;
}

export function CreateServerModal({ onClose }: Props) {
  const [name, setName] = useState('');
  const [iconUrl, setIconUrl] = useState('');
  const createServer = useChatStore((s) => s.createServer);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    createServer(trimmed, iconUrl.trim() || undefined);
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-full max-w-md rounded-lg bg-bg-primary p-6 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-4 text-xl font-bold text-text-primary">Create a Server</h2>

        <form onSubmit={handleSubmit}>
          <label className="mb-1 block text-xs font-semibold uppercase tracking-wide text-text-muted">
            Server Name
          </label>
          <input
            ref={inputRef}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="My Awesome Server"
            className="mb-4 w-full rounded bg-bg-input px-3 py-2 text-text-primary placeholder-text-muted outline-none focus:ring-2 focus:ring-bg-accent"
            maxLength={100}
          />

          <label className="mb-1 block text-xs font-semibold uppercase tracking-wide text-text-muted">
            Icon URL (optional)
          </label>
          <input
            type="text"
            value={iconUrl}
            onChange={(e) => setIconUrl(e.target.value)}
            placeholder="https://example.com/icon.png"
            className="mb-6 w-full rounded bg-bg-input px-3 py-2 text-text-primary placeholder-text-muted outline-none focus:ring-2 focus:ring-bg-accent"
          />

          <div className="flex justify-end gap-3">
            <button
              type="button"
              onClick={onClose}
              className="rounded px-4 py-2 text-sm text-text-secondary hover:text-text-primary"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!name.trim()}
              className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:opacity-90 disabled:opacity-50"
            >
              Create
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
