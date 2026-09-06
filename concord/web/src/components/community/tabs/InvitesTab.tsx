import { useState } from 'react';
import type { InviteInfo } from '../../../api/types';
import { ActionOutcome } from './ActionOutcome';
import { useActionStatus } from './useActionStatus';

// ── Invites Tab ──────────────────────────────────────────

export function InvitesTab({ invites, serverId, onCreate, onDelete }: {
  invites: InviteInfo[];
  serverId: string;
  onCreate: (serverId: string, maxUses?: number, expiresAt?: string, channelId?: string) => Promise<void>;
  onDelete: (serverId: string, inviteId: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [maxUses, setMaxUses] = useState('');
  const [expiresIn, setExpiresIn] = useState('24'); // hours
  const [copied, setCopied] = useState<string | null>(null);
  const { pending, outcome, run } = useActionStatus();

  const handleCreate = () => {
    const mu = maxUses ? parseInt(maxUses, 10) : undefined;
    const hours = parseInt(expiresIn, 10);
    const ea = hours > 0 ? new Date(Date.now() + hours * 3600000).toISOString() : undefined;
    void run('create', () => onCreate(serverId, mu, ea), 'Invite created.', () => {
      setShowForm(false);
      setMaxUses('');
      setExpiresIn('24');
    });
  };

  const copyCode = (code: string) => {
    navigator.clipboard.writeText(code);
    setCopied(code);
    setTimeout(() => setCopied(null), 2000);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Server Invites</h3>
        <button
          disabled={pending !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
        >
          {showForm ? 'Cancel' : 'Create Invite'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <div>
            <label htmlFor="invite-max-uses" className="block text-xs font-medium text-text-muted mb-1">Max Uses (0 = unlimited)</label>
            <input
              id="invite-max-uses"
              type="number"
              value={maxUses}
              onChange={e => setMaxUses(e.target.value)}
              placeholder="0"
              min="0"
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            />
          </div>
          <div>
            <label htmlFor="invite-expiry" className="block text-xs font-medium text-text-muted mb-1">Expires In (hours, 0 = never)</label>
            <select
              id="invite-expiry"
              value={expiresIn}
              onChange={e => setExpiresIn(e.target.value)}
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            >
              <option value="1">1 hour</option>
              <option value="6">6 hours</option>
              <option value="12">12 hours</option>
              <option value="24">24 hours</option>
              <option value="168">7 days</option>
              <option value="720">30 days</option>
              <option value="0">Never</option>
            </select>
          </div>
          <button
            disabled={pending !== null}
            onClick={handleCreate}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
          >
            {pending === 'create' ? 'Generating…' : 'Generate Invite'}
          </button>
        </div>
      )}

      {invites.length === 0 ? (
        <p className="text-text-muted text-sm">No active invites.</p>
      ) : (
        <div className="space-y-2">
          {invites.map(invite => (
            <div key={invite.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <code className="text-sm font-mono text-text-primary">{invite.code}</code>
                  <button
                    onClick={() => copyCode(invite.code)}
                    className="rounded bg-bg-tertiary px-2 py-0.5 text-xs text-text-muted hover:text-text-primary"
                  >
                    {copied === invite.code ? 'Copied!' : 'Copy'}
                  </button>
                </div>
                <div className="mt-1 text-xs text-text-muted">
                  Uses: {invite.use_count}{invite.max_uses ? ` / ${invite.max_uses}` : ' (unlimited)'}
                  {invite.expires_at && (
                    <span className="ml-2">
                      Expires: {new Date(invite.expires_at).toLocaleDateString()}
                    </span>
                  )}
                  <span className="ml-2">Created by: {invite.created_by}</span>
                </div>
              </div>
              <button
                disabled={pending !== null}
                onClick={() => run(`delete:${invite.id}`, () => onDelete(serverId, invite.id), 'Invite deleted.')}
                className="ml-2 rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700"
              >
                {pending === `delete:${invite.id}` ? 'Deleting…' : 'Delete'}
              </button>
            </div>
          ))}
        </div>
      )}
      <ActionOutcome outcome={outcome} />
    </div>
  );
}
