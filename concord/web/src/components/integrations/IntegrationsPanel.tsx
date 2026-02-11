import { useState, useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import type { WebhookInfo, SlashCommandInfo } from '../../api/types';

type Tab = 'webhooks' | 'commands' | 'bots' | 'oauth';

interface Props {
  serverId: string;
  onClose: () => void;
}

export function IntegrationsPanel({ serverId, onClose }: Props) {
  const [activeTab, setActiveTab] = useState<Tab>('webhooks');

  const webhooks = useChatStore(s => s.webhooks[serverId] ?? []);
  const slashCommands = useChatStore(s => s.slashCommands[serverId] ?? []);
  const botTokens = useChatStore(s => s.botTokens);
  const oauth2Apps = useChatStore(s => s.oauth2Apps);
  const channels = useChatStore(s => s.channels[serverId] ?? []);

  const listWebhooks = useChatStore(s => s.listWebhooks);
  const createWebhook = useChatStore(s => s.createWebhook);
  const deleteWebhook = useChatStore(s => s.deleteWebhook);
  const listSlashCommands = useChatStore(s => s.listSlashCommands);
  const deleteSlashCommand = useChatStore(s => s.deleteSlashCommand);
  const listOAuth2Apps = useChatStore(s => s.listOAuth2Apps);
  const createOAuth2App = useChatStore(s => s.createOAuth2App);
  const deleteOAuth2App = useChatStore(s => s.deleteOAuth2App);
  const createBot = useChatStore(s => s.createBot);

  useEffect(() => {
    if (activeTab === 'webhooks') listWebhooks(serverId);
    if (activeTab === 'commands') listSlashCommands(serverId);
    if (activeTab === 'oauth') listOAuth2Apps();
  }, [serverId, activeTab, listWebhooks, listSlashCommands, listOAuth2Apps]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose]);

  const tabLabels: Record<Tab, string> = {
    webhooks: 'Webhooks',
    commands: 'Commands',
    bots: 'Bots',
    oauth: 'OAuth Apps',
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-full max-w-3xl max-h-[85vh] flex flex-col rounded-lg bg-bg-primary shadow-xl" onClick={e => e.stopPropagation()}>
        <div className="flex items-center justify-between border-b border-border p-4">
          <h2 className="text-lg font-bold text-text-primary">Integrations</h2>
          <button onClick={onClose} className="text-text-muted hover:text-text-primary text-xl leading-none">&times;</button>
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
              onCreate={createWebhook}
              onDelete={deleteWebhook}
            />
          )}
          {activeTab === 'commands' && (
            <CommandsTab commands={slashCommands} onDelete={deleteSlashCommand} />
          )}
          {activeTab === 'bots' && (
            <BotsTab botTokens={botTokens} onCreate={createBot} />
          )}
          {activeTab === 'oauth' && (
            <OAuthTab apps={oauth2Apps} onCreate={createOAuth2App} onDelete={deleteOAuth2App} />
          )}
        </div>
      </div>
    </div>
  );
}

// ── Webhooks Tab ──

function WebhooksTab({ webhooks, serverId, channels, onCreate, onDelete }: {
  webhooks: WebhookInfo[];
  serverId: string;
  channels: { id: string; name: string }[];
  onCreate: (serverId: string, channelId: string, name: string, webhookType: string, url?: string) => void;
  onDelete: (webhookId: string) => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [channelId, setChannelId] = useState('');
  const [webhookType, setWebhookType] = useState<'incoming' | 'outgoing'>('incoming');
  const [url, setUrl] = useState('');

  const handleCreate = () => {
    if (!name.trim() || !channelId) return;
    onCreate(serverId, channelId, name.trim(), webhookType, webhookType === 'outgoing' ? url : undefined);
    setName('');
    setChannelId('');
    setUrl('');
    setShowForm(false);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Webhooks</h3>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90"
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
            onClick={handleCreate}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90"
          >
            Create
          </button>
        </div>
      )}

      {webhooks.length === 0 ? (
        <p className="text-text-muted text-sm">No webhooks configured.</p>
      ) : (
        <div className="space-y-2">
          {webhooks.map(wh => (
            <div key={wh.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-text-primary text-sm">{wh.name}</span>
                  <span className={`rounded px-1.5 py-0.5 text-xs ${wh.webhook_type === 'incoming' ? 'bg-green-900/30 text-green-400' : 'bg-blue-900/30 text-blue-400'}`}>
                    {wh.webhook_type}
                  </span>
                </div>
                <div className="mt-1 text-xs text-text-muted truncate">
                  Token: <code className="bg-bg-tertiary px-1 rounded">{wh.token}</code>
                </div>
              </div>
              <button
                onClick={() => onDelete(wh.id)}
                className="ml-2 text-red-400 hover:text-red-300 text-xs"
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Commands Tab ──

function CommandsTab({ commands, onDelete }: {
  commands: SlashCommandInfo[];
  onDelete: (commandId: string) => void;
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
          {commands.map(cmd => (
            <div key={cmd.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-text-primary text-sm">/{cmd.name}</span>
                </div>
                <p className="text-xs text-text-muted mt-0.5">{cmd.description || 'No description'}</p>
                {cmd.options.length > 0 && (
                  <div className="mt-1 flex gap-1 flex-wrap">
                    {cmd.options.map(opt => (
                      <span key={opt.name} className="rounded bg-bg-tertiary px-1.5 py-0.5 text-xs text-text-muted">
                        {opt.name}{opt.required ? '*' : ''}
                      </span>
                    ))}
                  </div>
                )}
              </div>
              <button
                onClick={() => onDelete(cmd.id)}
                className="ml-2 text-red-400 hover:text-red-300 text-xs"
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Bots Tab ──

function BotsTab({ botTokens, onCreate }: {
  botTokens: { id: string; name: string; scopes: string; created_at: string; last_used?: string | null }[];
  onCreate: (username: string) => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [botName, setBotName] = useState('');

  const handleCreate = () => {
    if (!botName.trim()) return;
    onCreate(botName.trim());
    setBotName('');
    setShowForm(false);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">Bot Accounts</h3>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90"
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
            onClick={handleCreate}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90"
          >
            Create Bot
          </button>
        </div>
      )}

      {botTokens.length === 0 ? (
        <p className="text-text-muted text-sm">No bot tokens. Create a bot to get started.</p>
      ) : (
        <div className="space-y-2">
          {botTokens.map(t => (
            <div key={t.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
              <div>
                <span className="font-medium text-text-primary text-sm">{t.name}</span>
                <span className="ml-2 text-xs text-text-muted">Scopes: {t.scopes}</span>
                {t.last_used && <span className="ml-2 text-xs text-text-muted">Last used: {new Date(t.last_used).toLocaleDateString()}</span>}
              </div>
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
  onCreate: (name: string, description: string, redirectUris: string, scopes?: string) => void;
  onDelete: (appId: string) => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [redirectUris, setRedirectUris] = useState('');

  const handleCreate = () => {
    if (!name.trim() || !redirectUris.trim()) return;
    onCreate(name.trim(), description.trim(), redirectUris.trim());
    setName('');
    setDescription('');
    setRedirectUris('');
    setShowForm(false);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text-secondary">OAuth2 Applications</h3>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90"
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
          <button
            onClick={handleCreate}
            className="rounded bg-bg-accent px-3 py-1.5 text-xs font-medium text-white hover:opacity-90"
          >
            Create App
          </button>
        </div>
      )}

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
                onClick={() => onDelete(app.id)}
                className="ml-2 text-red-400 hover:text-red-300 text-xs"
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
