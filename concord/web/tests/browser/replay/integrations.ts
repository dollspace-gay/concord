import { expect, test } from '@playwright/test';
import { type ChatStore, type UiStore } from './fixtures';

export function registerLifecycleCommandsResolveOnlyFromCorrelatedAcceptanceAndNeverAutoReplay() {
  test('lifecycle commands resolve only from correlated acceptance and never auto-replay', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(async () => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: Array<{ type?: string; request_id?: string; name?: string }> = [];
      store.setState({
        connected: true,
        activeAccountId: 'account-a',
        accountGeneration: 7,
        ws: {
          send: (command: unknown) => {
            sent.push(structuredClone(command) as typeof sent[number]);
            return true;
          }
        },
      });

      const first = store.getState().runLifecycleCommand(
        { type: 'create_server', name: 'Preserved input', icon_url: null },
        'server:create',
      );
      const duplicateError = await store.getState().runLifecycleCommand(
        { type: 'create_server', name: 'Duplicate', icon_url: null },
        'server:create',
      ).then(() => null, (error: Error) => error.message);
      const firstRequest = sent[0].request_id!;
      store.getState().handleEvent({
        type: 'command_error', request_id: firstRequest, code: 'DEPENDENCY_UNAVAILABLE',
        message: 'Storage unavailable', retryable: true,
      });
      const firstError = await first.then(() => null, (error: Error) => error.message);
      const sendsAfterRetryableError = sent.length;

      const explicitRetry = store.getState().runLifecycleCommand(
        { type: 'create_server', name: 'Preserved input', icon_url: null },
        'server:create',
      );
      const retryRequest = sent.at(-1)!.request_id!;
      store.getState().handleEvent({ type: 'lifecycle_command_succeeded', request_id: retryRequest });
      await explicitRetry;

      const wrongAccount = store.getState().runLifecycleCommand(
        { type: 'create_server', name: 'Scoped', icon_url: null },
        'server:create',
      );
      const wrongAccountRequest = sent.at(-1)!.request_id!;
      store.setState({ activeAccountId: 'account-b', accountGeneration: 8 });
      store.getState().handleEvent({
        type: 'lifecycle_command_succeeded', request_id: wrongAccountRequest,
      });
      const accountError = await wrongAccount.then(() => null, (error: Error) => error.message);

      return {
        duplicateError,
        firstError,
        sendsAfterRetryableError,
        sent,
        accountError,
      };
    });

    expect(result.duplicateError).toBe('This action is already pending or the pending action limit was reached.');
    expect(result.firstError).toBe('Storage unavailable');
    expect(result.sendsAfterRetryableError).toBe(1);
    expect(result.sent).toHaveLength(3);
    expect(result.sent[0]).toMatchObject({
      type: 'lifecycle_command', request_id: expect.any(String),
      command: { type: 'create_server', name: 'Preserved input' },
    });
    expect(result.sent[1].request_id).not.toBe(result.sent[0].request_id);
    expect(result.accountError).toBe('Account changed before the action completed.');
  });
}

export function registerRichBotResponsesRenderSafeEmbedsAndInvokeAccessibleControls() {
  test('rich bot responses render safe embeds and invoke accessible controls', async ({ page }) => {
    let externalImageRequests = 0;
    await page.route('https://images.example.test/**', async (route) => {
      externalImageRequests += 1;
      await route.fulfill({ status: 200, contentType: 'image/gif', body: Buffer.from('R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==', 'base64') });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const ui = (window as typeof window & { uiStore: UiStore }).uiStore;
      const sent: unknown[] = [];
      (window as typeof window & { componentCommands?: unknown[] }).componentCommands = sent;
      ui.setState({ activeServer: 'server', activeChannel: '#general', activeDirectConversation: null });
      store.setState({
        nickname: 'laurelai', activeAccountId: 'user', connected: true,
        ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
        messages: {
          'server:#general': [{
            id: 'response-message', from: 'helper-bot', sender_id: 'bot', content: 'Choose an action',
            timestamp: '2026-09-06T12:00:00Z',
            rich_embeds: [{
              title: 'Verified result', description: '**Ready**', color: '#228855',
              url: 'https://example.test/result', image_url: 'https://images.example.test/result.gif',
              thumbnail_url: 'https://user:secret@images.example.test/private.gif',
              fields: [{ name: 'Status', value: 'Complete', inline: true }],
            }],
            components: [{
              type: 'action_row', components: [
                { type: 'button', custom_id: 'confirm', label: 'Confirm', style: 'success' },
                {
                  type: 'select_menu', custom_id: 'priority', placeholder: 'Priority', options: [
                    { label: 'High', value: 'high' }, { label: 'Low', value: 'low' },
                  ]
                },
              ]
            }],
          }]
        },
      });
    });
    await expect(page.getByRole('article', { name: 'Embed: Verified result' })).toBeVisible();
    const resultLink = page.getByRole('link', { name: 'Verified result' });
    await expect(resultLink).toHaveAttribute('href', 'https://example.test/result');
    await expect(resultLink).toHaveAttribute('rel', 'noopener noreferrer');
    await expect(page.locator('img[src^="javascript:"]')).toHaveCount(0);
    expect(externalImageRequests).toBe(0);
    await expect(page.getByText('Loading it shares your IP address with that site.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Load external image: embed image' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Load external image: embed thumbnail' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Load external image: embed image' }).click();
    await expect.poll(() => externalImageRequests).toBe(1);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.setState({ activeAccountId: 'other-reader' });
    });
    await expect(page.getByRole('button', { name: 'Load external image: embed image' })).toBeVisible();
    expect(externalImageRequests).toBe(1);
    await page.getByRole('button', { name: 'Confirm' }).click();
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent = (window as typeof window & { componentCommands: Array<{ request_id?: string }> }).componentCommands;
      store.getState().handleEvent({ type: 'interaction_invoked', request_id: sent.at(-1)!.request_id! });
    });
    await page.getByRole('combobox', { name: 'Priority' }).selectOption('high');
    const commands = await page.evaluate(() => (window as typeof window & { componentCommands: unknown[] }).componentCommands);
    expect(commands).toEqual(expect.arrayContaining([
      expect.objectContaining({ type: 'invoke_message_component', message_id: 'response-message', custom_id: 'confirm', values: [] }),
      expect.objectContaining({ type: 'invoke_message_component', message_id: 'response-message', custom_id: 'priority', values: ['high'] }),
    ]));
  });
}

export function registerRetryableCommandErrorsResendTheIdenticalCorrelatedCommandOnce() {
  test('retryable command errors resend the identical correlated command once', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(async () => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: unknown[] = [];
      store.setState({
        nickname: 'carmilla',
        operationGeneration: 'generation-2',
        ws: { send: (command: unknown) => { sent.push(structuredClone(command)); return true; } },
      });
      const accepted = store.getState().sendMessage('server-1', 'general', 'hello');
      const first = sent[0] as { request_id: string };
      store.getState().handleEvent({
        type: 'command_error', request_id: first.request_id, code: 'DEPENDENCY_UNAVAILABLE',
        message: 'retry', retryable: true,
      });
      await new Promise((resolve) => setTimeout(resolve, 300));
      store.getState().handleEvent({
        type: 'command_error', request_id: first.request_id, code: 'DEPENDENCY_UNAVAILABLE',
        message: 'retry', retryable: true,
      });
      await new Promise((resolve) => setTimeout(resolve, 300));
      const beforeCommit = Object.keys(store.getState().pendingCommands);
      store.getState().handleEvent({
        type: 'command_committed', receipt: {
          request_id: first.request_id, client_message_id: first.request_id, message_id: 'm1',
          entity_version: 1, persisted_at: '2026-01-01T00:00:00Z', sequence: '1', replayed: false,
        },
      });
      return {
        accepted,
        sent,
        beforeCommit,
        afterCommit: Object.keys(store.getState().pendingCommands),
      };
    });
    expect(result.accepted).toBe(true);
    expect(result.sent).toHaveLength(2);
    expect(result.sent[1]).toEqual(result.sent[0]);
    expect(result.beforeCommit).toEqual([(result.sent[0] as { request_id: string }).request_id]);
    expect(result.afterCommit).toEqual([]);
  });
}

export function registerAccountChangeCancelsADelayedPrivateCommandRetry() {
  test('account change cancels a delayed private-command retry', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const sentByB = await page.evaluate(async () => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sentA: unknown[] = [];
      const sentB: unknown[] = [];
      store.setState({
        nickname: 'account-a', accountGeneration: 4, operationGeneration: 'generation-a',
        ws: { send: (command: unknown) => { sentA.push(command); return true; }, disconnect: () => { } },
      });
      store.getState().sendMessage('server-1', 'private', 'account A secret');
      const requestId = (sentA[0] as { request_id: string }).request_id;
      store.getState().handleEvent({
        type: 'command_error', request_id: requestId, code: 'DEPENDENCY_UNAVAILABLE',
        message: 'retry', retryable: true,
      });
      store.getState().disconnect();
      store.setState({
        nickname: 'account-b', ws: { send: (command: unknown) => { sentB.push(command); return true; } },
      });
      await new Promise((resolve) => setTimeout(resolve, 300));
      return sentB;
    });
    expect(sentByB).toEqual([]);
  });
}

export function registerAPostReconnectSnapshotReplaysAnUnacknowledgedCommandWithTheSameId() {
  test('a post-reconnect snapshot replays an unacknowledged command with the same id', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const sent = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const commands: unknown[] = [];
      store.setState({
        nickname: 'carmilla', accountGeneration: 7, operationGeneration: 'generation-2',
        ws: { send: (command: unknown) => { commands.push(structuredClone(command)); return true; } },
      });
      store.getState().sendMessage('server-1', 'general', 'recover me');
      store.getState().handleEvent({
        type: 'sync_snapshot', request_id: 'sync-reconnect', snapshot: {
          cursor: 'cursor-2', operation_generation: 'generation-2', protocol_version: 2,
          history_before: {}, messages: [], reactions: [], read_states: [],
        },
      });
      return commands;
    });
    expect(sent).toHaveLength(2);
    expect(sent[1]).toEqual(sent[0]);
  });
}

export function registerV2DurableDeliveryIsIdempotentAcrossBothLegacyEventOrdersAndStaleTombstoneUpdates() {
  test('v2 durable delivery is idempotent across both legacy event orders and stale tombstone updates', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const conversation = 'channel:6368616E6E656C2D31';
      store.setState({ nickname: 'carmilla' });
      store.getState().handleEvent({
        type: 'channel_list', server_id: 'server-1', channels: [{
          id: 'channel-1', conversation_id: conversation, server_id: 'server-1', name: 'general',
          topic: '', member_count: 2, category_id: null, position: 0, is_private: false,
          channel_type: 'text', thread_parent_message_id: null, archived: false,
          slowmode_seconds: 0, is_nsfw: false,
        }],
      });
      store.getState().handleEvent({
        type: 'sync_snapshot', request_id: 'sync', snapshot: {
          cursor: 'cursor', operation_generation: 'generation-2', protocol_version: 2,
          history_before: {}, messages: [], reactions: [], read_states: [],
        },
      });
      const message = (id: string, version: number, content: string, deleted = false) => ({
        conversation_id: conversation, descriptor: {}, entity_id: id, entity_type: 'message',
        entity_version: version, kind: deleted ? 'message_deleted' : 'message_created',
        message: {
          attachments: [], content, content_format: 'plain', conversation_id: conversation,
          created_at: '2026-01-01T00:00:00Z', deleted, edited_at: null, entity_version: version,
          mentions: [], message_id: id, reply_to: null, reply_to_id: null, sender_id: 'alice-id',
          sender_nick: 'alice', sequence: id === 'm1' ? '1' : '2',
        }, reaction: null, read_state: null,
      });
      const legacy = (id: string, content: string) => ({
        type: 'message', id, server_id: 'server-1', from: 'alice', target: 'general', content,
        timestamp: '2026-01-01T00:00:00Z', reply_to: null, attachments: null,
      });

      // Legacy then durable, and durable then legacy.
      store.getState().handleEvent(legacy('m1', 'legacy-first'));
      store.getState().handleEvent({ type: 'durable_event', event: message('m1', 1, 'canonical-one') });
      store.getState().handleEvent({ type: 'durable_event', event: message('m2', 1, 'canonical-two') });
      store.getState().handleEvent(legacy('m2', 'legacy-last'));
      // Duplicate, tombstone, then stale durable and legacy edits.
      store.getState().handleEvent({ type: 'durable_event', event: message('m2', 1, 'duplicate') });
      store.getState().handleEvent({ type: 'durable_event', event: message('m1', 3, '', true) });
      store.getState().handleEvent({ type: 'durable_event', event: message('m1', 2, 'stale-edit') });
      store.getState().handleEvent({
        type: 'message_edit', id: 'm1', server_id: 'server-1', channel: 'general',
        content: 'stale-legacy-edit', edited_at: '2026-01-01T00:01:00Z',
      });
      const state = store.getState();
      return {
        messages: state.messages['server-1:general'].map((entry: { id: string; content: string }) => ({ id: entry.id, content: entry.content })),
        unread: state.unreadCounts['server-1:general'],
        version: state.entityVersions['message:m1'],
      };
    });
    expect(result).toEqual({
      messages: [{ id: 'm1', content: '' }, { id: 'm2', content: 'canonical-two' }],
      unread: 2,
      version: 3,
    });
  });
}
