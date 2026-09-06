import { useState, useEffect, useCallback, useRef } from 'react';
import { useChatStore } from '../../stores/chatStore';
import type { BotAccountInfo, BotTokenInfo, WebhookDeliveryStatus, WebhookInfo, SlashCommandInfo } from '../../api/types';
import { Dialog } from '../Dialog';

type Tab = 'webhooks' | 'commands' | 'bots' | 'oauth';

interface Props {
  serverId: string;
  onClose: () => void;
}

const EMPTY_WEBHOOKS: WebhookInfo[] = [];
const EMPTY_COMMANDS: SlashCommandInfo[] = [];
const EMPTY_CHANNELS: { id: string; name: string }[] = [];

export function IntegrationsPanel({ serverId, onClose }: Props) {
  const [activeTab, setActiveTab] = useState<Tab>('webhooks');

  const webhooks = useChatStore(s => s.webhooks[serverId] ?? EMPTY_WEBHOOKS);
  const slashCommands = useChatStore(s => s.slashCommands[serverId] ?? EMPTY_COMMANDS);
  const botTokens = useChatStore(s => s.botTokens);
  const botAccounts = useChatStore(s => s.botAccounts);
  const botCredential = useChatStore(s => s.botCredential);
  const oauth2Apps = useChatStore(s => s.oauth2Apps);
  const channels = useChatStore(s => s.channels[serverId] ?? EMPTY_CHANNELS);
  const connected = useChatStore(s => s.connected);
  const accountGeneration = useChatStore(s => s.accountGeneration);

  const listWebhooks = useChatStore(s => s.listWebhooks);
  const listSlashCommands = useChatStore(s => s.listSlashCommands);
  const listOAuth2Apps = useChatStore(s => s.listOAuth2Apps);
  const listOwnedBots = useChatStore(s => s.listOwnedBots);
  const listBotTokens = useChatStore(s => s.listBotTokens);
  const clearBotCredential = useChatStore(s => s.clearBotCredential);
  const runLifecycleCommand = useChatStore(s => s.runLifecycleCommand);

  useEffect(() => {
    if (!connected) return;
    if (activeTab === 'webhooks') listWebhooks(serverId);
    if (activeTab === 'commands') listSlashCommands(serverId);
    if (activeTab === 'oauth') listOAuth2Apps();
    if (activeTab === 'bots') listOwnedBots();
  }, [serverId, activeTab, connected, listWebhooks, listSlashCommands, listOAuth2Apps, listOwnedBots]);

  const tabLabels: Record<Tab, string> = {
    webhooks: 'Webhooks',
    commands: 'Commands',
    bots: 'Bots',
    oauth: 'OAuth Apps',
  };

  const close = () => {
    clearBotCredential();
    onClose();
  };

  return (
    <Dialog label="Integrations" onClose={close} panelClassName="w-full max-w-3xl max-h-[85vh] flex flex-col rounded-lg bg-bg-primary shadow-xl">
        <div className="flex items-center justify-between border-b border-border p-4">
          <h2 className="text-lg font-bold text-text-primary">Integrations</h2>
          <button onClick={close} className="text-text-muted hover:text-text-primary text-xl leading-none">&times;</button>
        </div>

        <div className="flex border-b border-border">
          {(Object.keys(tabLabels) as Tab[]).map(t => (
            <button
              key={t}
              onClick={() => setActiveTab(t)}
              className={`px-4 py-2 text-sm font-medium ${
                activeTab === t ? 'border-b-2 border-bg-accent text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
            >
              {tabLabels[t]}
            </button>
          ))}
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {activeTab === 'webhooks' && (
            <WebhooksTab
              webhooks={webhooks}
              serverId={serverId}
              channels={channels}
              onCreate={(channelId, name, webhookType, url) => runLifecycleCommand({
                type: 'create_webhook', server_id: serverId, channel_id: channelId,
                name, webhook_type: webhookType, url,
              }, `webhook:create:${serverId}`)}
              onDelete={(webhookId) => runLifecycleCommand(
                { type: 'delete_webhook', webhook_id: webhookId },
                `webhook:delete:${webhookId}`,
              )}
            />
          )}
          {activeTab === 'commands' && (
            <CommandsTab commands={slashCommands} onDelete={(commandId) => runLifecycleCommand(
              { type: 'delete_slash_command', command_id: commandId },
              `command:delete:${commandId}`,
            )} />
          )}
          {activeTab === 'bots' && (
            <BotsTab
              key={`${accountGeneration}:${serverId}`}
              serverId={serverId}
              accountGeneration={accountGeneration}
              connected={connected}
              bots={botAccounts}
              botTokens={botTokens}
              credential={botCredential}
              onCreate={(username) => runLifecycleCommand(
                { type: 'create_bot', username }, 'bot:create',
              )}
              onSelect={listBotTokens}
              onCreateToken={(botUserId, name, scopes) => runLifecycleCommand(
                { type: 'create_bot_token', bot_user_id: botUserId, name: name ?? 'Bot token', scopes },
                `bot-token:create:${botUserId}`,
              )}
              onRevokeToken={(tokenId) => runLifecycleCommand(
                { type: 'delete_bot_token', token_id: tokenId }, `bot-token:revoke:${tokenId}`,
              )}
              onInstall={(botUserId) => runLifecycleCommand(
                { type: 'add_bot_to_server', bot_user_id: botUserId, server_id: serverId },
                `bot-install:${botUserId}:${serverId}`,
              )}
              onRemove={(botUserId) => runLifecycleCommand(
                { type: 'remove_bot_from_server', bot_user_id: botUserId, server_id: serverId },
                `bot-install:${botUserId}:${serverId}`,
              )}
              onDismissCredential={clearBotCredential}
            />
          )}
          {activeTab === 'oauth' && (
            <OAuthTab
              apps={oauth2Apps}
              onCreate={(name, description, redirectUris, clientType) => runLifecycleCommand({
                type: 'create_o_auth2_app', name, description,
                redirect_uris: redirectUris.split(',').map((uri) => uri.trim()).filter(Boolean),
                client_type: clientType,
              }, 'oauth-app:create')}
              onDelete={(appId) => runLifecycleCommand(
                { type: 'delete_o_auth2_app', app_id: appId }, `oauth-app:delete:${appId}`,
              )}
            />
          )}
        </div>
    </Dialog>
  );
}

function useLifecycleFeedback(scope: string) {
  const [pendingKey, setPendingKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const generation = useRef(0);

  const [previousScope, setPreviousScope] = useState(scope);
  if (previousScope !== scope) {
    setPreviousScope(scope);
    setPendingKey(null);
    setError(null);
    setSuccess(null);
  }

  useEffect(() => {
    generation.current += 1;
    return () => { generation.current += 1; };
  }, [scope]);

  const run = useCallback(async (
    key: string,
    action: () => Promise<void>,
    successMessage: string,
    afterSuccess?: () => void,
  ) => {
    const started = generation.current;
    setPendingKey(key);
    setError(null);
    setSuccess(null);
    try {
      await action();
      if (generation.current !== started) return;
      afterSuccess?.();
      setSuccess(successMessage);
    } catch (reason) {
      if (generation.current !== started) return;
      setError(reason instanceof Error ? reason.message : 'The action could not be completed.');
    } finally {
      if (generation.current === started) setPendingKey(null);
    }
  }, []);

  return { pendingKey, error, success, run };
}

function Feedback({ error, success }: { error: string | null; success: string | null }) {
  return (
    <>
      {error && <p role="alert" className="text-xs text-red-400">{error}</p>}
      {success && <p role="status" className="text-xs text-green-400">{success}</p>}
    </>
  );
}

// ── Webhooks Tab ──

function WebhooksTab({ webhooks, serverId, channels, onCreate, onDelete }: {
  webhooks: WebhookInfo[];
  serverId: string;
  channels: { id: string; name: string }[];
  onCreate: (channelId: string, name: string, webhookType: string, url?: string) => Promise<void>;
  onDelete: (webhookId: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [channelId, setChannelId] = useState('');
  const [webhookType, setWebhookType] = useState<'incoming' | 'outgoing'>('incoming');
  const [url, setUrl] = useState('');
  const feedback = useLifecycleFeedback(serverId);

  const handleCreate = async () => {
    if (!name.trim() || !channelId) return;
    await feedback.run(
      'create',
      () => onCreate(channelId, name.trim(), webhookType, webhookType === 'outgoing' ? url.trim() : undefined),
      'Webhook created.',
      () => {
        setName('');
        setChannelId('');
        setWebhookType('incoming');
        setUrl('');
        setShowForm(false);
      },
    );
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Webhooks</h3>
        <button
          disabled={feedback.pendingKey !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {showForm ? 'Cancel' : 'Create Webhook'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Webhook name"
            value={name}
            onChange={e => setName(e.target.value)}
          />
          <select
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none"
            value={channelId}
            onChange={e => setChannelId(e.target.value)}
          >
            <option value="">Select channel...</option>
            {channels.map(ch => (
              <option key={ch.id} value={ch.id}>{ch.name}</option>
            ))}
          </select>
          <select
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none"
            value={webhookType}
            onChange={e => setWebhookType(e.target.value as 'incoming' | 'outgoing')}
          >
            <option value="incoming">Incoming</option>
            <option value="outgoing">Outgoing</option>
          </select>
          {webhookType === 'outgoing' && (
            <input
              className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
              placeholder="Outgoing URL"
              value={url}
              onChange={e => setUrl(e.target.value)}
            />
          )}
          <button
            disabled={feedback.pendingKey !== null}
            onClick={() => void handleCreate()}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {feedback.pendingKey === 'create' ? 'Creating…' : 'Create'}
          </button>
        </div>
      )}

      <Feedback error={feedback.error} success={feedback.success} />

      {webhooks.length === 0 ? (
        <p className="text-text-muted text-sm">No webhooks configured.</p>
      ) : (
        <div className="space-y-2">
          {webhooks.map(wh => <WebhookCard key={wh.id} webhook={wh} onDelete={onDelete} />)}
        </div>
      )}
    </div>
  );
}

function WebhookCard({ webhook, onDelete }: { webhook: WebhookInfo; onDelete: (id: string) => Promise<void> }) {
  const [deliveries, setDeliveries] = useState<WebhookDeliveryStatus[]>([]);
  const [loading, setLoading] = useState(webhook.webhook_type === 'outgoing');
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const outgoing = webhook.webhook_type === 'outgoing';

  const refresh = useCallback((signal?: AbortSignal) => {
    if (!outgoing) return Promise.resolve();
    const generation = useChatStore.getState().accountGeneration;
    return fetch(`/api/webhooks/${encodeURIComponent(webhook.id)}/deliveries?limit=10`, { signal })
      .then(async (response) => {
        if (!response.ok) throw new Error('Delivery status is unavailable');
        return await response.json() as { deliveries: WebhookDeliveryStatus[] };
      })
      .then((body) => {
        if (signal?.aborted || useChatStore.getState().accountGeneration !== generation) return;
        setDeliveries(body.deliveries);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (signal?.aborted || useChatStore.getState().accountGeneration !== generation) return;
        setError(reason instanceof Error ? reason.message : 'Delivery status is unavailable');
      })
      .finally(() => {
        if (!signal?.aborted && useChatStore.getState().accountGeneration === generation) setLoading(false);
      });
  }, [outgoing, webhook.id]);

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh]);

  const test = async () => {
    const generation = useChatStore.getState().accountGeneration;
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`/api/webhooks/${encodeURIComponent(webhook.id)}/test`, { method: 'POST' });
      if (!response.ok) throw new Error('Test delivery could not be queued.');
      await refresh();
    } catch (reason) {
      if (useChatStore.getState().accountGeneration !== generation) return;
      setError(reason instanceof Error && reason.message === 'Test delivery could not be queued.'
        ? reason.message
        : 'The test delivery result is unknown; refresh status before retrying.');
    } finally {
      if (useChatStore.getState().accountGeneration === generation) setLoading(false);
    }
  };

  const retry = async (deliveryId: string) => {
    const generation = useChatStore.getState().accountGeneration;
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`/api/webhook-deliveries/${encodeURIComponent(deliveryId)}/retry`, { method: 'POST' });
      if (!response.ok) throw new Error('Delivery could not be retried.');
      await refresh();
    } catch (reason) {
      if (useChatStore.getState().accountGeneration !== generation) return;
      setError(reason instanceof Error && reason.message === 'Delivery could not be retried.'
        ? reason.message
        : 'The retry result is unknown; refresh status before retrying again.');
    } finally {
      if (useChatStore.getState().accountGeneration === generation) setLoading(false);
    }
  };

  const remove = async () => {
    setDeleting(true);
    setError(null);
    try {
      await onDelete(webhook.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Webhook deletion failed.');
    } finally {
      setDeleting(false);
    }
  };

  const copyIncomingUrl = async (incomingUrl: string) => {
    try {
      await navigator.clipboard.writeText(incomingUrl);
      setCopyStatus('Webhook URL copied.');
    } catch {
      setCopyStatus('Clipboard access failed. Select and copy the URL manually.');
    }
  };

  const incomingUrl = webhook.token
    ? `${window.location.origin}/api/webhooks/${webhook.id}/${webhook.token}`
    : null;
  return (
    <div className="rounded bg-bg-secondary p-3 space-y-2">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 min-w-0">
          <span className="font-medium text-text-primary text-sm truncate">{webhook.name}</span>
          <span className={`rounded px-1.5 py-0.5 text-xs ${outgoing ? 'bg-blue-900/30 text-blue-400' : 'bg-green-900/30 text-green-400'}`}>
            {webhook.webhook_type}
          </span>
        </div>
        <div className="flex gap-2">
          {outgoing && <button disabled={loading} onClick={() => void test()} className="text-xs text-blue-400 disabled:opacity-50">Test</button>}
          <button disabled={deleting} onClick={() => void remove()} className="text-red-400 hover:text-red-300 text-xs disabled:opacity-50">
            {deleting ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
      {!outgoing && incomingUrl && (
        <div className="text-xs text-amber-300">
          Credential shown once. <button className="underline" onClick={() => void copyIncomingUrl(incomingUrl)}>Copy webhook URL</button>
          {copyStatus && <div role={copyStatus.startsWith('Clipboard') ? 'alert' : 'status'}>{copyStatus}</div>}
        </div>
      )}
      {!outgoing && !incomingUrl && <div className="text-xs text-text-muted">Credential hidden. Create a new webhook to rotate it.</div>}
      {outgoing && webhook.token && (
        <div className="text-xs text-amber-300">
          <div>Signing secret shown once. Save it before closing this dialog.</div>
          <code className="mt-1 block break-all rounded bg-bg-tertiary p-2 text-text-primary select-all">{webhook.token}</code>
        </div>
      )}
      {outgoing && !webhook.token && <div className="text-xs text-text-muted">Signing secret hidden. Create a new webhook to rotate it.</div>}
      {outgoing && (
        <div className="space-y-1 text-xs">
          {error && <div className="text-red-400">{error}</div>}
          {!error && deliveries.length === 0 && <div className="text-text-muted">No deliveries yet.</div>}
          {deliveries.map(delivery => (
            <div key={delivery.delivery_id} className="flex justify-between gap-2 text-text-muted">
              <span>{delivery.event_type} · {delivery.state} · attempt {delivery.attempt_count}</span>
              {delivery.state === 'failed' && <button className="text-blue-400" disabled={loading} onClick={() => void retry(delivery.delivery_id)}>Retry</button>}
            </div>
          ))}
          <button className="text-blue-400 disabled:opacity-50" disabled={loading} onClick={() => { setLoading(true); void refresh(); }}>{loading ? 'Loading…' : 'Refresh status'}</button>
        </div>
      )}
    </div>
  );
}

// ── Commands Tab ──

function CommandsTab({ commands, onDelete }: {
  commands: SlashCommandInfo[];
  onDelete: (commandId: string) => Promise<void>;
}) {
  return (
    <div className="space-y-4">
      <h3 className="text-sm font-semibold text-text-secondary">Slash Commands</h3>
      <p className="text-xs text-text-muted">
        Commands are registered by bots. Use the Bot API to register slash commands.
      </p>

      {commands.length === 0 ? (
        <p className="text-text-muted text-sm">No slash commands registered.</p>
      ) : (
        <div className="space-y-2">
          {commands.map(cmd => <CommandCard key={cmd.id} command={cmd} onDelete={onDelete} />)}
        </div>
      )}
    </div>
  );
}

function CommandCard({ command, onDelete }: {
  command: SlashCommandInfo;
  onDelete: (commandId: string) => Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remove = async () => {
    setPending(true);
    setError(null);
    try {
      await onDelete(command.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Command deletion failed.');
    } finally {
      setPending(false);
    }
  };
  return (
    <div className="rounded bg-bg-secondary p-3">
      <div className="flex items-center justify-between">
        <div className="min-w-0 flex-1">
          <span className="font-medium text-text-primary text-sm">/{command.name}</span>
          <p className="text-xs text-text-muted mt-0.5">{command.description || 'No description'}</p>
          {command.options.length > 0 && (
            <div className="mt-1 flex gap-1 flex-wrap">
              {command.options.map(opt => (
                <span key={opt.name} className="rounded bg-bg-tertiary px-1.5 py-0.5 text-xs text-text-muted">
                  {opt.name}{opt.required ? '*' : ''}
                </span>
              ))}
            </div>
          )}
        </div>
        <button disabled={pending} onClick={() => void remove()} className="ml-2 text-red-400 hover:text-red-300 text-xs disabled:opacity-50">
          {pending ? 'Deleting…' : 'Delete'}
        </button>
      </div>
      {error && <p role="alert" className="mt-2 text-xs text-red-400">{error}</p>}
    </div>
  );
}

// ── Bots Tab ──

function BotsTab({ serverId, accountGeneration, connected, bots, botTokens, credential, onCreate, onSelect, onCreateToken, onRevokeToken, onInstall, onRemove, onDismissCredential }: {
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

// ── OAuth Apps Tab ──

function OAuthTab({ apps, onCreate, onDelete }: {
  apps: { id: string; name: string; description: string; scopes: string; is_public: boolean; created_at: string }[];
  onCreate: (name: string, description: string, redirectUris: string, clientType: 'confidential' | 'public') => Promise<void>;
  onDelete: (appId: string) => Promise<void>;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [redirectUris, setRedirectUris] = useState('');
  const [clientType, setClientType] = useState<'confidential' | 'public'>('confidential');
  const feedback = useLifecycleFeedback('oauth-apps');

  const handleCreate = async () => {
    if (!name.trim() || !redirectUris.trim()) return;
    await feedback.run(
      'create',
      () => onCreate(name.trim(), description.trim(), redirectUris.trim(), clientType),
      'OAuth application created.',
      () => {
        setName('');
        setDescription('');
        setRedirectUris('');
        setShowForm(false);
      },
    );
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">OAuth2 Applications</h3>
        <button
          disabled={feedback.pendingKey !== null}
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {showForm ? 'Cancel' : 'Create App'}
        </button>
      </div>

      {showForm && (
        <div className="rounded bg-bg-secondary p-3 space-y-3">
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="App name"
            value={name}
            onChange={e => setName(e.target.value)}
          />
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Description"
            value={description}
            onChange={e => setDescription(e.target.value)}
          />
          <input
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="Redirect URIs (comma separated)"
            value={redirectUris}
            onChange={e => setRedirectUris(e.target.value)}
          />
          <select
            aria-label="OAuth client type"
            className="w-full rounded bg-bg-tertiary px-3 py-1.5 text-sm text-text-primary outline-none"
            value={clientType}
            onChange={e => setClientType(e.target.value as 'confidential' | 'public')}
          >
            <option value="confidential">Confidential client (server-side)</option>
            <option value="public">Public client (browser or native app)</option>
          </select>
          <p className="text-xs text-text-muted">Available scopes: identify, servers.read</p>
          <button
            disabled={feedback.pendingKey !== null}
            onClick={() => void handleCreate()}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {feedback.pendingKey === 'create' ? 'Creating…' : 'Create App'}
          </button>
        </div>
      )}

      <Feedback error={feedback.error} success={feedback.success} />

      {apps.length === 0 ? (
        <p className="text-text-muted text-sm">No OAuth2 applications.</p>
      ) : (
        <div className="space-y-2">
          {apps.map(app => (
            <div key={app.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-text-primary text-sm">{app.name}</span>
                  {app.is_public && <span className="rounded bg-green-900/30 px-1.5 py-0.5 text-xs text-green-400">Public</span>}
                </div>
                <p className="text-xs text-text-muted mt-0.5">{app.description || 'No description'}</p>
                <div className="text-xs text-text-muted mt-0.5">
                  Client ID: <code className="bg-bg-tertiary px-1 rounded">{app.id}</code>
                </div>
              </div>
              <button
                disabled={feedback.pendingKey !== null}
                onClick={() => void feedback.run(
                  `delete:${app.id}`,
                  () => onDelete(app.id),
                  'OAuth application deleted.',
                )}
                className="ml-2 text-red-400 hover:text-red-300 text-xs disabled:opacity-50"
              >
                {feedback.pendingKey === `delete:${app.id}` ? 'Deleting…' : 'Delete'}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
