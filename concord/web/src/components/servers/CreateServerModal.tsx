import { useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { Dialog } from '../Dialog';

interface Props {
  onClose: () => void;
}

export function CreateServerModal({ onClose }: Props) {
  const [name, setName] = useState('');
  const [iconUrl, setIconUrl] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const createServer = useChatStore((s) => s.createServer);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    setPending(true);
    setError(null);
    try {
      await createServer(trimmed, iconUrl.trim() || undefined);
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Server creation was rejected.');
    } finally {
      setPending(false);
    }
  };

  return (
    <Dialog label="Create a Server" onClose={onClose} panelClassName="w-full max-w-md rounded-lg bg-bg-primary p-6 shadow-xl">
        <h2 className="mb-4 text-xl font-bold text-text-primary">Create a Server</h2>

        <form onSubmit={handleSubmit}>
          <label className="mb-1 block text-xs font-semibold uppercase tracking-wide text-text-muted">
            Server Name
          </label>
          <input
            data-dialog-initial-focus
            type="text"
            value={name}
            disabled={pending}
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
            disabled={pending}
            onChange={(e) => setIconUrl(e.target.value)}
            placeholder="https://example.com/icon.png"
            className="mb-6 w-full rounded bg-bg-input px-3 py-2 text-text-primary placeholder-text-muted outline-none focus:ring-2 focus:ring-bg-accent"
          />

          <div className="flex justify-end gap-3">
            <button
              type="button"
              onClick={onClose}
              disabled={pending}
              className="rounded px-4 py-2 text-sm text-text-secondary hover:text-text-primary"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!name.trim() || pending}
              className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:opacity-90 disabled:opacity-50"
            >
              {pending ? 'Creating…' : 'Create'}
            </button>
          </div>
          {error && <p role="alert" className="mt-3 text-sm text-red-400">{error}</p>}
        </form>
    </Dialog>
  );
}
