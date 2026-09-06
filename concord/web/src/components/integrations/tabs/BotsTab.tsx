import { useEffect, useState } from 'react';
import type { BotAccountInfo, BotTokenInfo } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';
import { Feedback } from './Feedback';
import { useLifecycleFeedback } from './useLifecycleFeedback';

// ── Bots Tab ──

export function BotsTab({ serverId, accountGeneration, connected, bots, botTokens, credential, onCreate, onSelect, onCreateToken, onRevokeToken, onInstall, onRemove, onDismissCredential }: {
  serverId: string;
  accountGeneration: number;
  connected: boolean;
  bots: BotAccountInfo[];
  botTokens: BotTokenInfo[];
  credential: { botUserId: string; token: string; credential: BotTokenInfo } | null;
  onCreate: (username: string) => Promise<void>;
  onSelect: (botUserId: string) => void;
  onCreateToken: (botUserId: string, name?: string, scopes?: string) => Promise<void>;
  onRevokeToken: (tokenId: string) => Promise<void>;
  onInstall: (botUserId: string) => Promise<void>;
  onRemove: (botUserId: string) => Promise<void>;
  onDismissCredential: () => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [botName, setBotName] = useState('');
  const [selectedBotId, setSelectedBotId] = useState('');
  const [tokenName, setTokenName] = useState('Bot token');
  const [tokenScopes, setTokenScopes] = useState('bot messages');
  const [credentialOrigin, setCredentialOrigin] = useState<{ token: string; serverId: string } | null>(null);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [installationIntent, setInstallationIntent] = useState<'install' | 'remove' | null>(null);
  const feedback = useLifecycleFeedback(`${accountGeneration}:${serverId}`);

  const effectiveBotId = selectedBotId || credential?.botUserId || bots[0]?.id || '';
  const selectedBot = bots.find((bot) => bot.id === effectiveBotId);
  useEffect(() => {
    if (connected && effectiveBotId) onSelect(effectiveBotId);
  }, [connected, effectiveBotId, onSelect]);

  const bindLatestCredential = (botUserId?: string) => {
    const latest = useChatStore.getState().botCredential;
    if (!latest || (botUserId && latest.botUserId !== botUserId)) return;
    setSelectedBotId(latest.botUserId);
    setCredentialOrigin({ token: latest.token, serverId });
    setCopyStatus(null);
  };

  const handleCreate = async () => {
    if (!botName.trim()) return;
    await feedback.run(
      'create',
      () => onCreate(botName.trim()),
      'Bot account created.',
      () => {
        bindLatestCredential();
        setInstallationIntent('install');
        setBotName('');
        setShowForm(false);
      },
    );
  };

  const runSelected = async (key: string, action: () => Promise<void>, message: string) => {
    await feedback.run(key, action, message);
  };

  const runInstallation = async (botUserId: string, action: 'install' | 'remove') => {
    await feedback.run(
      'installation',
      () => action === 'install' ? onInstall(botUserId) : onRemove(botUserId),
      action === 'install' ? 'Bot installed on server.' : 'Bot removed from server.',
      () => setInstallationIntent(action === 'install' ? 'remove' : 'install'),
    );
  };

  const visibleCredential = credential
    && credentialOrigin?.token === credential.token
    && credentialOrigin.serverId === serverId
    && selectedBot?.id === credential.botUserId
    ? credential
    : null;

  const copyCredential = async () => {
    if (!visibleCredential) return;
    try {
      await navigator.clipboard.writeText(visibleCredential.token);
      setCopyStatus('Bot token copied.');
    } catch {
      setCopyStatus('Clipboard access failed. Select and copy the token manually.');
    }
  };

  return (
    <div className="space-y-4">
      {visibleCredential && (
        <div role="status" className="rounded border border-yellow-500/60 bg-yellow-500/10 p-3 text-sm text-text-primary">
          <div className="font-semibold">Copy this bot token now. It will not be shown again.</div>
          <code className="mt-2 block break-all rounded bg-bg-tertiary p-2 select-all">{visibleCredential.token}</code>
          <div className="mt-2 flex gap-2">
            <button className="rounded bg-bg-accent px-2 py-1 text-xs text-white" onClick={() => void copyCredential()}>Copy token</button>
            <button className="rounded bg-bg-tertiary px-2 py-1 text-xs" onClick={() => {
              setCredentialOrigin(null);
              setCopyStatus(null);
              onDismissCredential();
            }}>I saved it</button>
          </div>
          {copyStatus && <p role={copyStatus.startsWith('Clipboard') ? 'alert' : 'status'} className="mt-2 text-xs">{copyStatus}</p>}
        </div>
      )}
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Bot Accounts</h3>
        <button
          disabled={feedback.pendingKey !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {showForm ? 'Cancel' : 'Create Bot'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Bot username"
            value={botName}
            onChange={e => setBotName(e.target.value)}
          />
          <button
            disabled={feedback.pendingKey !== null}
            onClick={() => void handleCreate()}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {feedback.pendingKey === 'create' ? 'Creating…' : 'Create Bot'}
          </button>
        </div>
      )}

      <Feedback error={feedback.error} success={feedback.success} />

      {bots.length === 0 && !effectiveBotId ? (
        <p className="text-text-muted text-sm">No bot accounts. Create one to get started.</p>
      ) : (
        <div className="space-y-3">
          <label className="block text-xs text-text-muted">Bot account
            <select className="mt-1 w-full rounded bg-bg-tertiary px-3 py-2 text-sm text-text-primary" value={effectiveBotId} onChange={(event) => {
              const next = bots.find((bot) => bot.id === event.target.value);
              setSelectedBotId(event.target.value);
              setInstallationIntent(next?.installed_server_ids.includes(serverId) ? 'remove' : 'install');
            }}>
              {!selectedBot && effectiveBotId && <option value={effectiveBotId}>Selected bot (reconnecting…)</option>}
              {bots.map((bot) => <option key={bot.id} value={bot.id}>{bot.username}</option>)}
            </select>
          </label>
          {effectiveBotId && (
            <div className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div>
                <div className="font-medium text-text-primary">{selectedBot?.username ?? 'Selected bot'}</div>
                <div className="text-xs text-text-muted">{selectedBot ? `ID: ${selectedBot.id}` : 'Details will refresh after reconnecting.'}</div>
              </div>
              {(selectedBot?.installed_server_ids.includes(serverId) || (!selectedBot && installationIntent === 'remove')) ? (
                <button disabled={feedback.pendingKey !== null} className="rounded bg-red-500/20 px-3 py-1 text-xs text-red-300 disabled:opacity-50" onClick={() => void runInstallation(effectiveBotId, 'remove')}>
                  {feedback.pendingKey === 'installation' ? 'Removing…' : 'Remove from server'}
                </button>
              ) : (
                <button disabled={feedback.pendingKey !== null} className="rounded bg-bg-accent px-3 py-1 text-xs text-white disabled:opacity-50" onClick={() => void runInstallation(effectiveBotId, 'install')}>
                  {feedback.pendingKey === 'installation' ? 'Installing…' : 'Install on server'}
                </button>
              )}
            </div>
          )}
          {selectedBot && <div className="grid gap-2 rounded bg-bg-secondary p-3 sm:grid-cols-[1fr_1fr_auto]">
            <input aria-label="Token name" className="rounded bg-bg-tertiary px-3 py-2 text-sm" value={tokenName} onChange={(event) => setTokenName(event.target.value)} />
            <input aria-label="Token scopes" className="rounded bg-bg-tertiary px-3 py-2 text-sm" value={tokenScopes} onChange={(event) => setTokenScopes(event.target.value)} />
            <button disabled={feedback.pendingKey !== null} className="rounded bg-bg-accent px-3 py-2 text-xs text-white disabled:opacity-50" onClick={() => void feedback.run(
              'token-create',
              () => onCreateToken(selectedBot.id, tokenName, tokenScopes),
              'Bot token created.',
              () => bindLatestCredential(selectedBot.id),
            )}>
              {feedback.pendingKey === 'token-create' ? 'Creating…' : 'Create token'}
            </button>
          </div>}
          {botTokens.map(t => (
            <div key={t.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div>
                <span className="font-medium text-text-primary text-sm">{t.name}</span>
                <span className="ml-2 text-xs text-text-muted">Scopes: {t.scopes}</span>
                {t.last_used && <span className="ml-2 text-xs text-text-muted">Last used: {new Date(t.last_used).toLocaleDateString()}</span>}
              </div>
              <button disabled={feedback.pendingKey !== null} className="ml-2 text-xs text-red-400 hover:text-red-300 disabled:opacity-50" onClick={() => void runSelected(`token-revoke:${t.id}`, () => onRevokeToken(t.id), 'Bot token revoked.')}>
                {feedback.pendingKey === `token-revoke:${t.id}` ? 'Revoking…' : 'Revoke'}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
