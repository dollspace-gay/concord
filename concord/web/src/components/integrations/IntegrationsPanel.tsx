import { useEffect, useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { Dialog } from '../Dialog';
import { BotsTab } from './tabs/BotsTab';
import { CommandsTab } from './tabs/CommandsTab';
import { OAuthTab } from './tabs/OAuthTab';
import { WebhooksTab } from './tabs/WebhooksTab';
import { EMPTY_CHANNELS, EMPTY_COMMANDS, EMPTY_WEBHOOKS } from './tabs/defaults';

type Tab = 'webhooks' | 'commands' | 'bots' | 'oauth';

interface Props {
  serverId: string;
  onClose: () => void;
}

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
            className={`px-4 py-2 text-sm font-medium ${activeTab === t ? 'border-b-2 border-bg-accent text-text-primary' : 'text-text-muted hover:text-text-secondary'
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
