import { useState } from 'react';
import type { ServerCommunityInfo } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';
import { ActionOutcome } from './ActionOutcome';
import { useActionStatus } from './useActionStatus';

// ── Discovery Tab ───────────────────────────────────────

export function DiscoveryTab({ servers, onJoin, onRefresh }: {
  servers: ServerCommunityInfo[];
  onJoin: (code: string) => Promise<void>;
  onRefresh: (category?: string) => void;
}) {
  const [filterCategory, setFilterCategory] = useState('');
  const [joinCode, setJoinCode] = useState('');
  const { pending, outcome, run } = useActionStatus();

  const handleJoinByCode = () => {
    if (!joinCode.trim()) return;
    const code = joinCode.trim();
    void run('join', () => onJoin(code), 'Invite accepted.', () => setJoinCode(''));
  };

  return (
    <div className="space-y-4">
      {/* Join by invite code */}
      <div className="rounded bg-bg-secondary p-3 space-y-2">
        <h3 className="text-sm font-semibold text-text-secondary">Join by Invite Code</h3>
        <div className="flex gap-2">
          <input
            aria-label="Invite code"
            type="text"
            value={joinCode}
            onChange={e => setJoinCode(e.target.value)}
            placeholder="Enter invite code..."
            className="flex-1 rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none focus:ring-1 focus:ring-bg-accent"
            onKeyDown={e => { if (e.key === 'Enter') handleJoinByCode(); }}
          />
          <button
            onClick={handleJoinByCode}
            disabled={!joinCode.trim() || pending !== null}
            className="rounded bg-bg-accent px-4 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {pending === 'join' ? 'Joining…' : 'Join'}
          </button>
        </div>
      </div>
      <ActionOutcome outcome={outcome} />

      {/* Browse servers */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text-secondary">Discover Servers</h3>
          <div className="flex items-center gap-2">
            <select
              aria-label="Discovery category"
              value={filterCategory}
              onChange={e => {
                setFilterCategory(e.target.value);
                onRefresh(e.target.value || undefined);
              }}
              className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-primary outline-none"
            >
              <option value="">All Categories</option>
              <option value="gaming">Gaming</option>
              <option value="music">Music</option>
              <option value="education">Education</option>
              <option value="science">Science & Technology</option>
              <option value="entertainment">Entertainment</option>
              <option value="community">General Community</option>
            </select>
            <button
              onClick={() => onRefresh(filterCategory || undefined)}
              className="rounded bg-bg-tertiary px-2 py-1 text-xs text-text-muted hover:text-text-primary"
            >
              Refresh
            </button>
          </div>
        </div>

        {servers.length === 0 ? (
          <p className="text-text-muted text-sm">No discoverable servers found.</p>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {servers.map(server => (
              <div key={server.server_id} className="rounded bg-bg-secondary p-4 flex flex-col justify-between">
                <div>
                  <div className="flex items-center gap-2">
                    <div className="h-10 w-10 rounded-full bg-bg-accent/30 flex items-center justify-center text-text-primary text-sm font-bold">
                      {(server.description ?? server.server_id).charAt(0).toUpperCase()}
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-medium text-text-primary truncate">
                        {server.server_id}
                      </p>
                      {server.category && (
                        <span className="rounded bg-bg-accent/20 px-1.5 py-0.5 text-xs text-bg-accent">
                          {server.category}
                        </span>
                      )}
                    </div>
                  </div>
                  {server.description && (
                    <p className="mt-2 text-xs text-text-secondary line-clamp-2">{server.description}</p>
                  )}
                </div>
                <button
                  onClick={() => {
                    // For discovery, use server_id to join
                    const store = useChatStore.getState();
                    store.joinServer(server.server_id);
                  }}
                  className="mt-3 w-full rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-bg-accent/80"
                >
                  Join Server
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
