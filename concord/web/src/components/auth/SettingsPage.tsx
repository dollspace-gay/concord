import { useEffect, useState } from 'react';
import { useAuthStore } from '../../stores/authStore';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import * as api from '../../api/client';
import type { IrcToken } from '../../api/types';

export function SettingsPage() {
  const user = useAuthStore((s) => s.user);
  const logout = useAuthStore((s) => s.logout);
  const setShowSettings = useUiStore((s) => s.setShowSettings);

  const [tokens, setTokens] = useState<IrcToken[]>([]);
  const [newTokenLabel, setNewTokenLabel] = useState('');
  const [newToken, setNewToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    api.getTokens().then(setTokens).catch(console.error);
  }, []);

  const handleCreateToken = async () => {
    setLoading(true);
    try {
      const result = await api.createToken(newTokenLabel || undefined);
      setNewToken(result.token);
      setNewTokenLabel('');
      const updated = await api.getTokens();
      setTokens(updated);
    } catch (e) {
      console.error('Failed to create token:', e);
    }
    setLoading(false);
  };

  const handleDeleteToken = async (id: string) => {
    try {
      await api.deleteToken(id);
      setTokens((prev) => prev.filter((t) => t.id !== id));
    } catch (e) {
      console.error('Failed to delete token:', e);
    }
  };

  const handleLogout = async () => {
    await logout();
    setShowSettings(false);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="max-h-[80vh] w-full max-w-lg overflow-y-auto rounded-lg bg-bg-secondary p-6">
        <div className="mb-6 flex items-center justify-between">
          <h2 className="text-xl font-bold text-text-primary">Settings</h2>
          <button
            onClick={() => setShowSettings(false)}
            className="rounded p-1 text-text-muted transition-colors hover:text-text-primary"
          >
            <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Profile */}
        {user && (
          <section className="mb-6">
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">
              Profile
            </h3>
            <div className="flex items-center gap-4 rounded-md bg-bg-tertiary p-4">
              {user.avatar_url ? (
                <img src={user.avatar_url} alt="" className="h-16 w-16 rounded-full" />
              ) : (
                <div className="flex h-16 w-16 items-center justify-center rounded-full bg-bg-accent text-xl font-bold text-white">
                  {user.username[0]?.toUpperCase()}
                </div>
              )}
              <div>
                <p className="text-lg font-semibold text-text-primary">{user.username}</p>
                {user.email && <p className="text-sm text-text-muted">{user.email}</p>}
              </div>
            </div>
          </section>
        )}

        {/* IRC Tokens */}
        <section className="mb-6">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">
            IRC Access Tokens
          </h3>
          <p className="mb-3 text-sm text-text-muted">
            Use these tokens to connect from IRC clients like HexChat, irssi, or WeeChat.
            Use the token as your password with <code className="text-text-secondary">PASS</code>.
          </p>

          {newToken && (
            <div className="mb-3 rounded-md border border-status-online/30 bg-status-online/10 p-3">
              <p className="mb-1 text-sm font-semibold text-status-online">
                Token created! Copy it now — it won't be shown again.
              </p>
              <code className="block break-all rounded bg-bg-primary p-2 text-sm text-text-primary">
                {newToken}
              </code>
              <button
                onClick={() => {
                  navigator.clipboard.writeText(newToken);
                }}
                className="mt-2 rounded bg-bg-accent px-3 py-1 text-sm text-white transition-colors hover:bg-bg-accent-hover"
              >
                Copy to clipboard
              </button>
            </div>
          )}

          <div className="mb-3 flex gap-2">
            <input
              type="text"
              value={newTokenLabel}
              onChange={(e) => setNewTokenLabel(e.target.value)}
              placeholder="Token label (optional)"
              className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            />
            <button
              onClick={handleCreateToken}
              disabled={loading}
              className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover disabled:opacity-50"
            >
              Generate
            </button>
          </div>

          {tokens.length > 0 && (
            <div className="space-y-2">
              {tokens.map((t) => (
                <div
                  key={t.id}
                  className="flex items-center justify-between rounded-md bg-bg-tertiary p-3"
                >
                  <div>
                    <p className="text-sm font-medium text-text-primary">
                      {t.label || 'Unnamed token'}
                    </p>
                    <p className="text-xs text-text-muted">
                      Created {new Date(t.created_at).toLocaleDateString()}
                      {t.last_used && ` · Last used ${new Date(t.last_used).toLocaleDateString()}`}
                    </p>
                  </div>
                  <button
                    onClick={() => handleDeleteToken(t.id)}
                    className="rounded px-3 py-1 text-sm text-bg-danger transition-colors hover:bg-bg-danger/10"
                  >
                    Revoke
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* AT Protocol Settings */}
        <AtprotoSyncSection />

        {/* Logout */}
        <section>
          <button
            onClick={handleLogout}
            className="w-full rounded bg-bg-danger px-4 py-2 font-medium text-white transition-colors hover:bg-bg-danger/80"
          >
            Log Out
          </button>
        </section>
      </div>
    </div>
  );
}

function AtprotoSyncSection() {
  const atprotoSyncEnabled = useChatStore((s) => s.atprotoSyncEnabled);
  const fetchAtprotoSyncSetting = useChatStore((s) => s.fetchAtprotoSyncSetting);
  const setAtprotoSyncEnabled = useChatStore((s) => s.setAtprotoSyncEnabled);
  const [toggling, setToggling] = useState(false);

  useEffect(() => {
    fetchAtprotoSyncSetting();
  }, [fetchAtprotoSyncSetting]);

  const handleToggle = async () => {
    setToggling(true);
    try {
      await setAtprotoSyncEnabled(!atprotoSyncEnabled);
    } catch (e) {
      console.error('Failed to toggle AT Protocol sync:', e);
    }
    setToggling(false);
  };

  return (
    <section className="mb-6">
      <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-text-muted">
        AT Protocol
      </h3>
      <div className="rounded-md bg-bg-tertiary p-4">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-text-primary">
              Sync messages to AT Protocol
            </p>
            <p className="mt-1 text-xs text-text-muted">
              Write your messages as records on your PDS for data portability.
            </p>
          </div>
          <button
            onClick={handleToggle}
            disabled={toggling}
            className={`relative h-6 w-11 rounded-full transition-colors ${
              atprotoSyncEnabled ? 'bg-blue-500' : 'bg-bg-input'
            } ${toggling ? 'opacity-50' : ''}`}
          >
            <span
              className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
                atprotoSyncEnabled ? 'translate-x-5' : ''
              }`}
            />
          </button>
        </div>
      </div>
    </section>
  );
}
