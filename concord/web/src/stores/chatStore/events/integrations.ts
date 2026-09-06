import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import type { HistoryMessage, OAuth2AppInfo, SlashCommandInfo } from '../../../api/types';
import { channelKey } from '../../../api/types';
import { usePendingStore } from '../../pendingStore';
import { MAX_MESSAGES_PER_CHANNEL } from '../defaults';
import type { ChatStoreContext } from '../types';

export function handleIntegrationsEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'webhook_list' | 'webhook_update' | 'webhook_delete' | 'slash_command_list' | 'slash_command_update' | 'slash_command_delete' | 'interaction_create' | 'interaction_response' | 'interaction_invoked' | 'lifecycle_command_succeeded' | 'bot_token_list' | 'bot_account_list' | 'bot_credential_created' | 'o_auth2_app_list' | 'o_auth2_app_update' }>) {
  switch (event.type) {
    // ── Phase 8: Integrations & Bots ──
    case 'webhook_list':
      set({ webhooks: { ...get().webhooks, [event.server_id]: event.webhooks } });
      break;
    case 'webhook_update': {
      const existing = get().webhooks[event.server_id] || [];
      const idx = existing.findIndex(w => w.id === event.webhook.id);
      const updated = idx >= 0 ? [...existing.slice(0, idx), event.webhook, ...existing.slice(idx + 1)] : [...existing, event.webhook];
      set({ webhooks: { ...get().webhooks, [event.server_id]: updated } });
      break;
    }
    case 'webhook_delete':
      set({ webhooks: { ...get().webhooks, [event.server_id]: (get().webhooks[event.server_id] || []).filter(w => w.id !== event.webhook_id) } });
      break;
    case 'slash_command_list':
      set({
        slashCommands: {
          ...get().slashCommands,
          [event.server_id]: event.commands.map((command) => ({
            ...command,
            server_id: event.server_id,
            created_at: '',
            options: command.options.map((option) => ({
              ...option,
              required: option.required ?? false,
              choices: option.choices ?? undefined,
            })),
          })),
        },
      });
      break;
    case 'slash_command_update': {
      const existing = get().slashCommands[event.server_id] || [];
      const idx = existing.findIndex(c => c.id === event.command.id);
      const command: SlashCommandInfo = {
        ...event.command,
        server_id: event.server_id,
        created_at: '',
        options: event.command.options.map((option) => ({
          ...option,
          required: option.required ?? false,
          choices: option.choices ?? undefined,
        })),
      };
      const updated = idx >= 0 ? [...existing.slice(0, idx), command, ...existing.slice(idx + 1)] : [...existing, command];
      set({ slashCommands: { ...get().slashCommands, [event.server_id]: updated } });
      break;
    }
    case 'slash_command_delete':
      set({ slashCommands: { ...get().slashCommands, [event.server_id]: (get().slashCommands[event.server_id] || []).filter(c => c.id !== event.command_id) } });
      break;
    case 'interaction_create':
      // Interactions are ephemeral — log for debugging
      console.log('Interaction created:', event.interaction);
      break;
    case 'interaction_response':
      set((state) => {
        const key = channelKey(event.server_id, event.channel);
        const message: HistoryMessage = {
          id: `ephemeral:${event.interaction_id}`,
          from: 'Interaction',
          content: event.response.content ?? '',
          timestamp: new Date().toISOString(),
          rich_embeds: event.response.embeds ?? undefined,
          components: event.response.components ?? undefined,
        };
        const current = state.messages[key] ?? [];
        const index = current.findIndex((entry) => entry.id === message.id);
        const messages = index >= 0
          ? [...current.slice(0, index), message, ...current.slice(index + 1)]
          : [...current, message].slice(-MAX_MESSAGES_PER_CHANNEL);
        return { messages: { ...state.messages, [key]: messages } };
      });
      break;
    case 'interaction_invoked': {
      const pending = usePendingStore.getState().takeInteraction(event.request_id);
      if (pending) {
        if (pending.accountGeneration === get().accountGeneration) pending.resolve();
        else pending.reject(new Error('Account changed before the interaction completed.'));
      }
      break;
    }
    case 'lifecycle_command_succeeded': {
      const pending = usePendingStore.getState().takeLifecycle(event.request_id);
      if (pending) {
        if (pending.accountGeneration === get().accountGeneration && pending.connection === get().ws) pending.resolve();
        else pending.reject(new Error('Account changed before the action completed.'));
      }
      break;
    }
    case 'bot_token_list':
      set({ botTokens: event.tokens });
      break;
    case 'bot_account_list':
      set({ botAccounts: event.bots });
      break;
    case 'bot_credential_created':
      set((state) => ({
        botCredential: { botUserId: event.bot_user_id, token: event.token, credential: event.credential },
        botTokens: state.botAccounts.some((bot) => bot.id === event.bot_user_id)
          ? [event.credential, ...state.botTokens.filter((token) => token.id !== event.credential.id)]
          : state.botTokens,
      }));
      break;
    case 'o_auth2_app_list':
      set({ oauth2Apps: event.apps.map((app) => ({ ...app, redirect_uris: app.redirect_uris.join('\n') })) });
      break;
    case 'o_auth2_app_update': {
      const existing = get().oauth2Apps;
      const idx = existing.findIndex(a => a.id === event.app.id);
      const app: OAuth2AppInfo = { ...event.app, redirect_uris: event.app.redirect_uris.join('\n') };
      const updated = idx >= 0 ? [...existing.slice(0, idx), app, ...existing.slice(idx + 1)] : [...existing, app];
      set({ oauth2Apps: updated });
      break;
    }
  }
}
