import { usePendingStore } from '../pendingStore';
import type { ChatState, ChatStoreContext } from './types';

export function createIntegrationActions({ set, get }: ChatStoreContext): Pick<ChatState, 'createWebhook' | 'listWebhooks' | 'updateWebhook' | 'deleteWebhook' | 'createBot' | 'listOwnedBots' | 'clearBotCredential' | 'createBotToken' | 'listBotTokens' | 'deleteBotToken' | 'addBotToServer' | 'removeBotFromServer' | 'registerSlashCommand' | 'listSlashCommands' | 'deleteSlashCommand' | 'invokeSlashCommand' | 'invokeMessageComponent' | 'createOAuth2App' | 'listOAuth2Apps' | 'deleteOAuth2App'> {
  return {
    // ── Phase 8: Integrations & Bots ──
    createWebhook: (serverId, channelId, name, webhookType, url) => {
      get().ws?.send({ type: 'create_webhook', server_id: serverId, channel_id: channelId, name, webhook_type: webhookType, url });
    },

    listWebhooks: (serverId) => {
      get().ws?.send({ type: 'list_webhooks', server_id: serverId });
    },

    updateWebhook: (webhookId, name, avatarUrl) => {
      const webhook = Object.values(get().webhooks).flat().find((candidate) => candidate.id === webhookId);
      if (!webhook) return;
      get().ws?.send({ type: 'update_webhook', webhook_id: webhookId, channel_id: webhook.channel_id, name, avatar_url: avatarUrl });
    },

    deleteWebhook: (webhookId) => {
      get().ws?.send({ type: 'delete_webhook', webhook_id: webhookId });
    },

    createBot: (username) => {
      get().ws?.send({ type: 'create_bot', username });
    },

    listOwnedBots: () => {
      get().ws?.send({ type: 'list_owned_bots' });
    },

    clearBotCredential: () => set({ botCredential: null }),

    createBotToken: (botUserId, name, scopes) => {
      get().ws?.send({
        type: 'create_bot_token',
        bot_user_id: botUserId,
        name: name ?? 'Bot token',
        ...(scopes === undefined ? {} : { scopes }),
      });
    },

    listBotTokens: (botUserId) => {
      get().ws?.send({ type: 'list_bot_tokens', bot_user_id: botUserId });
    },

    deleteBotToken: (tokenId) => {
      get().ws?.send({ type: 'delete_bot_token', token_id: tokenId });
    },

    addBotToServer: (botUserId, serverId) => {
      get().ws?.send({ type: 'add_bot_to_server', bot_user_id: botUserId, server_id: serverId });
    },

    removeBotFromServer: (botUserId, serverId) => {
      get().ws?.send({ type: 'remove_bot_from_server', bot_user_id: botUserId, server_id: serverId });
    },

    registerSlashCommand: (serverId, name, description, optionsJson) => {
      get().ws?.send({ type: 'register_slash_command', server_id: serverId, name, description, options_json: optionsJson });
    },

    listSlashCommands: (serverId) => {
      get().ws?.send({ type: 'list_slash_commands', server_id: serverId });
    },

    deleteSlashCommand: (commandId) => {
      get().ws?.send({ type: 'delete_slash_command', command_id: commandId });
    },

    invokeSlashCommand: (serverId, channelId, commandName, argsJson) => {
      const ws = get().ws;
      if (!ws) return Promise.reject(new Error('Not connected.'));
      const requestId = crypto.randomUUID();
      const result = new Promise<void>((resolve, reject) => {
        usePendingStore.getState().registerInteraction(requestId, { accountGeneration: get().accountGeneration, resolve, reject });
      });
      if (!ws.send({ type: 'invoke_slash_command', request_id: requestId, server_id: serverId, channel: channelId, command_name: commandName, args_json: argsJson })) {
        usePendingStore.getState().takeInteraction(requestId);
        return Promise.reject(new Error('Interaction was not sent; reconnecting.'));
      }
      return result;
    },

    invokeMessageComponent: (messageId, customId, values = []) => {
      const ws = get().ws;
      if (!ws) return Promise.reject(new Error('Not connected.'));
      const requestId = crypto.randomUUID();
      const result = new Promise<void>((resolve, reject) => {
        usePendingStore.getState().registerInteraction(requestId, { accountGeneration: get().accountGeneration, resolve, reject });
      });
      if (!ws.send({ type: 'invoke_message_component', request_id: requestId, message_id: messageId, custom_id: customId, values })) {
        usePendingStore.getState().takeInteraction(requestId);
        return Promise.reject(new Error('Interaction was not sent; reconnecting.'));
      }
      return result;
    },

    createOAuth2App: (name, description, redirectUris, clientType) => {
      get().ws?.send({ type: 'create_o_auth2_app', name, description, redirect_uris: redirectUris.split(',').map((uri) => uri.trim()).filter(Boolean), client_type: clientType });
    },

    listOAuth2Apps: () => {
      get().ws?.send({ type: 'list_o_auth2_apps' });
    },

    deleteOAuth2App: (appId) => {
      get().ws?.send({ type: 'delete_o_auth2_app', app_id: appId });
    }
  };
}
