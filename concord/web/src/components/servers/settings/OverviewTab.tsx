import { useState } from 'react';
import { ExternalImage } from '../../ExternalImage';

// ── Overview Tab ──────────────────────────────────────────

export function OverviewTab({
  serverId,
  server,
  updateServer,
  setVanityCode,
  deleteServer,
  close,
}: {
  serverId: string;
  server: { name: string; icon_url?: string | null; role?: string | null };
  updateServer: (serverId: string, name?: string, iconUrl?: string) => Promise<void>;
  setVanityCode: (serverId: string, vanityCode?: string | null) => Promise<void>;
  deleteServer: (serverId: string) => Promise<void>;
  close: () => void;
}) {
  const [name, setName] = useState(server.name);
  const [iconUrl, setIconUrl] = useState(server.icon_url ?? '');
  const [vanity, setVanity] = useState('');
  const [saved, setSaved] = useState(false);
  const [pending, setPending] = useState<'details' | 'vanity' | 'delete' | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (key: 'details' | 'vanity' | 'delete', action: () => Promise<void>, accepted?: () => void) => {
    if (pending) return;
    setPending(key);
    setError(null);
    setSaved(false);
    try {
      await action();
      accepted?.();
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'The change was rejected.');
    } finally {
      setPending(null);
    }
  };

  const handleSave = () => {
    void run('details', () => updateServer(serverId, name.trim() || undefined, iconUrl.trim() || undefined));
  };

  const handleVanitySave = () => {
    void run('vanity', () => setVanityCode(serverId, vanity.trim() || null));
  };

  return (
    <div className="space-y-4">
      <div>
        <label className="mb-1 block text-sm font-medium text-text-secondary">Server Name</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary outline-none"
          placeholder="Server name"
        />
      </div>
      <div>
        <label className="mb-1 block text-sm font-medium text-text-secondary">Icon URL</label>
        <input
          type="text"
          value={iconUrl}
          onChange={(e) => setIconUrl(e.target.value)}
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
          placeholder="https://example.com/icon.png"
        />
        {iconUrl && (
          <div className="mt-2 flex items-center gap-3">
            <ExternalImage
              src={iconUrl}
              alt="Server icon preview"
              label="server icon preview"
              privacyScopeKey={`server-settings:${serverId}:icon-preview`}
              className="h-12 w-12 rounded-full object-cover"
            />
            <span className="text-xs text-text-muted">Preview</span>
          </div>
        )}
      </div>
      <div className="flex items-center gap-3">
        <button
          disabled={pending !== null}
          onClick={handleSave}
          className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
        >
          {pending === 'details' ? 'Saving…' : 'Save Changes'}
        </button>
        {saved && <span className="text-sm text-green-400">Saved!</span>}
      </div>

      {/* Vanity Invite URL */}
      <div className="border-t border-border-primary pt-4">
        <label className="mb-1 block text-sm font-medium text-text-secondary">Vanity Invite Code</label>
        <p className="mb-2 text-xs text-text-muted">Set a custom invite code (e.g., &quot;my-server&quot;). 2-32 lowercase letters, digits, and hyphens.</p>
        <div className="flex gap-2">
          <input
            type="text"
            value={vanity}
            onChange={(e) => setVanity(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ''))}
            className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="my-server"
            maxLength={32}
          />
          <button
            disabled={pending !== null}
            onClick={handleVanitySave}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
          >
            {pending === 'vanity' ? 'Saving…' : 'Set'}
          </button>
          <button
            disabled={pending !== null}
            onClick={() => void run('vanity', () => setVanityCode(serverId, null), () => setVanity(''))}
            className="rounded px-3 py-2 text-sm text-text-muted hover:text-text-primary"
          >
            Clear
          </button>
        </div>
      </div>
      {error && <p role="alert" className="text-sm text-red-400">{error}</p>}
      {server.role === 'owner' && (
        <div className="border-t border-red-500/40 pt-4">
          <h3 className="mb-2 text-sm font-semibold text-red-400">Delete Server</h3>
          <p className="mb-2 text-xs text-text-muted">Permanently delete this server and its channels.</p>
          <button
            disabled={pending !== null}
            onClick={() => void run('delete', () => deleteServer(serverId), close)}
            className="rounded bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700"
          >
            {pending === 'delete' ? 'Deleting…' : 'Delete Server'}
          </button>
        </div>
      )}
    </div>
  );
}
