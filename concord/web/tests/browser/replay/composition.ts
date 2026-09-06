import { expect, test, type WebSocketRoute } from '@playwright/test';
import { type ChatStore, type UiStore } from './fixtures';

export function registerCanonicalPrivateUploadImagesLoadDirectlyWhileOtherLocalLookingURLsStayBlocked() {
  test('canonical private upload images load directly while other local-looking URLs stay blocked', async ({ page }) => {
    const requestedUrls: string[] = [];
    page.on('request', (request) => requestedUrls.push(request.url()));
    await page.route('**/api/uploads/*', async (route) => {
      await route.fulfill({ status: 200, contentType: 'image/gif', body: Buffer.from('R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==', 'base64') });
    });
    await page.goto('/layout-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.uiStore.setState({ activeServer: 'server', activeChannel: '#general', showMemberList: true });
      scope.chatStore.setState({
        servers: [{
          id: 'server', name: 'Private media', owner_id: 'owner', created_at: '2026-01-01T00:00:00Z',
          icon_url: '/api/uploads/10000000-0000-4000-8000-000000000001',
        }],
        members: {
          'server:#general': [
            { nickname: 'Local member', user_id: 'local', server_avatar_url: '/api/uploads/20000000-0000-4000-8000-000000000002' },
            { nickname: 'Relative path', user_id: 'relative', avatar_url: '/admin' },
            { nickname: 'Protocol relative', user_id: 'protocol-relative', avatar_url: '//images.example.test/avatar.gif' },
            { nickname: 'Private host', user_id: 'private-host', avatar_url: 'https://127.0.0.1/avatar.gif' },
            { nickname: 'Private scheme', user_id: 'private-scheme', avatar_url: 'javascript:alert(1)' },
            { nickname: 'Malformed upload', user_id: 'malformed-upload', avatar_url: '/api/uploads/not-a-uuid' },
          ]
        },
        userProfiles: {
          local: {
            id: 'local', username: 'Local member', created_at: '2026-01-01T00:00:00Z',
            avatar_url: '/api/uploads/30000000-0000-4000-8000-000000000003',
            banner_url: '/api/uploads/40000000-0000-4000-8000-000000000004',
          }
        },
      });
      scope.uiStore.getState().setShowUserProfile('local');
    });

    const localPaths = [
      '/api/uploads/10000000-0000-4000-8000-000000000001',
      '/api/uploads/20000000-0000-4000-8000-000000000002',
      '/api/uploads/30000000-0000-4000-8000-000000000003',
      '/api/uploads/40000000-0000-4000-8000-000000000004',
    ];
    for (const path of localPaths) await expect(page.locator(`img[src="${path}"]`)).toBeVisible();
    await expect.poll(() => localPaths.every((path) => requestedUrls.some((url) => new URL(url).pathname === path))).toBe(true);
    expect(requestedUrls.some((url) => new URL(url).pathname === '/admin')).toBe(false);
    expect(requestedUrls.some((url) => url.includes('images.example.test/avatar.gif'))).toBe(false);
    expect(requestedUrls.some((url) => url.includes('127.0.0.1/avatar.gif'))).toBe(false);
    await expect(page.locator('img[src^="javascript:"]')).toHaveCount(0);
    await expect(page.locator('img[src="/api/uploads/not-a-uuid"]')).toHaveCount(0);
  });
}

export function registerRejectedCommandInteractionsRetainDraftsAndRestoreComponentControls() {
  test('rejected command interactions retain drafts and restore component controls', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const ui = (window as typeof window & { uiStore: UiStore }).uiStore;
      const sent: unknown[] = [];
      (window as typeof window & { rejectedCommands?: unknown[] }).rejectedCommands = sent;
      ui.setState({ activeServer: 'server', activeChannel: '#general', activeDirectConversation: null });
      store.setState({
        nickname: 'laurelai', activeAccountId: 'user', connected: true,
        ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
        slashCommands: {
          server: [{
            id: 'fail-command', bot_user_id: 'bot', name: 'fail', description: 'Reject this command',
            options: [], created_at: '2026-09-06T12:00:00Z',
          }]
        },
        messages: {
          'server:#general': [{
            id: 'component-message', from: 'helper-bot', content: '', timestamp: '2026-09-06T12:00:00Z',
            components: [{
              type: 'action_row', components: [
                { type: 'button', custom_id: 'denied', label: 'Denied action', style: 'danger' },
              ]
            }],
          }]
        },
      });
    });
    const component = page.getByRole('button', { name: 'Denied action' });
    await component.click();
    await expect(component).toBeDisabled();
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const commands = (window as typeof window & { rejectedCommands: Array<{ type?: string; request_id?: string }> }).rejectedCommands;
      const request = commands.find((command) => command.type === 'invoke_message_component')!;
      store.getState().handleEvent({ type: 'command_error', request_id: request.request_id!, code: 'NOT_FOUND', message: 'Action expired', retryable: false });
    });
    await expect(component).toBeEnabled();
    await expect(page.getByRole('alert', { name: '' }).filter({ hasText: 'Action expired' })).toBeVisible();

    const composer = page.getByRole('textbox', { name: /Message/ });
    await composer.fill('/fail');
    await composer.press('Enter');
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const commands = (window as typeof window & { rejectedCommands: Array<{ type?: string; request_id?: string }> }).rejectedCommands;
      const request = commands.find((command) => command.type === 'invoke_slash_command')!;
      store.getState().handleEvent({ type: 'command_error', request_id: request.request_id!, code: 'FORBIDDEN', message: 'Command unavailable', retryable: false });
    });
    await expect(composer).toHaveValue('/fail');
    await expect(page.getByRole('alert').filter({ hasText: 'Command unavailable' })).toBeVisible();
  });
}

export function registerMountedMultilineComposerPreservesShiftEnterAndNeverSubmitsDuringIMEComposition() {
  test('mounted multiline composer preserves Shift+Enter and never submits during IME composition', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sentCommands: unknown[] };
      stores.sentCommands = [];
      stores.chatStore.setState({
        nickname: 'carmilla', activeAccountId: 'self', operationGeneration: 'generation',
        ws: { send: (command: unknown) => { stores.sentCommands.push(structuredClone(command)); return true; } },
      });
      stores.uiStore.getState().setActiveServer('server-1');
      stores.uiStore.getState().setActiveChannel('general');
    });
    const composer = page.getByPlaceholder('Message general');
    await composer.fill('first line');
    await composer.press('Shift+Enter');
    await composer.type('second line');
    await composer.dispatchEvent('compositionstart');
    await composer.press('Enter');
    await composer.dispatchEvent('compositionend');
    expect(await page.evaluate(() => (window as typeof window & { sentCommands: Array<{ type?: string }> }).sentCommands
      .filter((command) => command.type === 'send_message'))).toEqual([]);
    await composer.press('Enter');
    const sent = await page.evaluate(() => (window as typeof window & { sentCommands: Array<{ type: string; content: string }> }).sentCommands
      .filter((command) => command.type === 'send_message'));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ type: 'send_message', content: 'first line\nsecond line' });
  });
}

export function registerRealBrowserSocketSnapshotAndServerCloseClearProtectedDataWhilePreservingItsDraft() {
  test('real browser socket snapshot and server close clear protected data while preserving its draft', async ({ page }) => {
    let connectionCount = 0;
    let firstSocket: WebSocketRoute | null = null;
    await page.routeWebSocket(/\/ws\?nickname=/, (socket) => {
      connectionCount += 1;
      if (!firstSocket) firstSocket = socket;
      let advertisedChannel = false;
      socket.onMessage((raw) => {
        const command = JSON.parse(raw.toString()) as { type: string; subscriptions?: string[]; request_id?: string };
        if (command.type === 'list_servers') {
          socket.send(JSON.stringify({
            type: 'server_list', servers: [{ id: 'server-1', name: 'Private', member_count: 1 }],
          }));
        }
        if (command.type === 'sync' && !advertisedChannel) {
          advertisedChannel = true;
          socket.send(JSON.stringify({
            type: 'channel_list', server_id: 'server-1', channels: [{
              id: 'channel-1', conversation_id: 'channel:6368616E6E656C2D31', server_id: 'server-1',
              name: 'secret', topic: '', member_count: 1, category_id: null, position: 0,
              is_private: true, channel_type: 'text', thread_parent_message_id: null, archived: false,
              slowmode_seconds: 0, is_nsfw: false,
            }],
          }));
        } else if (command.type === 'sync' && command.subscriptions?.length) {
          socket.send(JSON.stringify({
            type: 'sync_snapshot', request_id: command.request_id, snapshot: {
              cursor: 'cursor', operation_generation: 'generation-2', protocol_version: 2,
              history_before: {}, reactions: [], read_states: [], messages: [{
                attachments: [], content: 'private history', content_format: 'plain',
                conversation_id: command.subscriptions[0], created_at: '2026-01-01T00:00:00Z',
                deleted: false, edited_at: null, entity_version: 1, mentions: [], message_id: 'private-1',
                reply_to: null, reply_to_id: null, sender_id: 'alice', sender_nick: 'alice', sequence: '1',
              }],
            },
          }));
        }
      });
    });

    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.getState().connect('carmilla');
    });
    await expect.poll(() => page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      return store.getState().messages['server-1:secret']?.[0]?.content;
    })).toBe('private history');
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.getState().setDraft('server-1:secret', 'unsent private draft');
    });

    await firstSocket!.close({ code: 4003, reason: 'credential revoked' });
    await expect.poll(() => page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const state = store.getState();
      return {
        connected: state.connected,
        servers: state.servers.length,
        messages: Object.keys(state.messages).length,
        draft: state.drafts['server-1:secret'],
      };
    })).toEqual({ connected: false, servers: 0, messages: 0, draft: 'unsent private draft' });
    await expect.poll(() => connectionCount, { timeout: 5_000 }).toBeGreaterThanOrEqual(2);
  });
}

export function registerLogoutIsolatesDraftsByDurableAccountAndRestoresThemOnReturn() {
  test('logout isolates drafts by durable account and restores them on return', async ({ page }) => {
    await page.routeWebSocket(/\/ws\?nickname=/, (socket) => socket.onMessage(() => { }));
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.getState().connect('alice-handle', 'account-a');
      store.getState().setDraft('server-1:secret', 'A private draft');
      store.getState().disconnect();
      store.getState().connect('bob-handle', 'account-b');
      const visibleToB = store.getState().drafts['server-1:secret'];
      store.getState().setDraft('server-2:general', 'B draft');
      store.getState().disconnect();
      store.getState().connect('alice-new-handle', 'account-a');
      return {
        visibleToB: visibleToB ?? null,
        restoredA: store.getState().drafts,
      };
    });
    expect(result).toEqual({
      visibleToB: null,
      restoredA: { 'server-1:secret': 'A private draft' },
    });
  });
}

export function registerAFailedSocketSendKeepsTheAccountDraftAndRemovesTheOptimisticRow() {
  test('a failed socket send keeps the account draft and removes the optimistic row', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.setState({
        nickname: 'carmilla', activeAccountId: 'account-a', operationGeneration: 'generation-2',
        drafts: { 'server-1:general': 'composition in progress' },
        ws: { send: () => false },
      });
      const accepted = store.getState().sendMessage('server-1', 'general', 'composition in progress');
      return {
        accepted,
        draft: store.getState().drafts['server-1:general'],
        messages: store.getState().messages['server-1:general'] ?? [],
      };
    });
    expect(result).toEqual({ accepted: false, draft: 'composition in progress', messages: [] });
  });
}

export function registerMountedComposerNeverSendsAnUploadThroughADifferentAccountSocket() {
  test('mounted composer never sends an upload through a different account socket', async ({ page }) => {
    let releaseUpload!: () => void;
    const uploadReleased = new Promise<void>((resolve) => { releaseUpload = resolve; });
    let markStarted!: () => void;
    const uploadStarted = new Promise<void>((resolve) => { markStarted = resolve; });
    await page.route(/\/api\/uploads/, async (route) => {
      markStarted();
      await uploadReleased;
      await route.fulfill({
        status: 201, contentType: 'application/json', body: JSON.stringify({
          id: 'attachment-1', filename: 'proof.txt', content_type: 'text/plain', file_size: 5,
          url: '/api/uploads/attachment-1',
        })
      });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sentCommands: unknown[] };
      stores.sentCommands = [];
      stores.chatStore.setState({
        nickname: 'alice', activeAccountId: 'account-a', accountGeneration: 1,
        operationGeneration: 'generation-a', ws: { send: (command: unknown) => { stores.sentCommands.push(command); return true; } },
      });
      stores.uiStore.getState().setActiveServer('server-1');
      stores.uiStore.getState().setActiveChannel('general');
    });
    const input = page.getByPlaceholder('Message general');
    await input.fill('account A composition');
    await page.locator('input[type=file]').setInputFiles({
      name: 'proof.txt', mimeType: 'text/plain', buffer: Buffer.from('proof'),
    });
    await input.press('Enter');
    await uploadStarted;
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sentCommands: unknown[] };
      stores.chatStore.setState({
        activeAccountId: 'account-b', accountGeneration: 2, drafts: {},
        operationGeneration: 'generation-b', ws: { send: (command: unknown) => { stores.sentCommands.push(command); return true; } },
      });
    });
    releaseUpload();
    await expect(page.getByPlaceholder('Message general')).toHaveValue('');
    await expect.poll(() => page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; sentCommands: Array<{ type?: string }> };
      return {
        files: scope.chatStore.getState().compositionFiles['account-a:server-1:general']?.length ?? 0,
        sends: scope.sentCommands.filter((command) => command.type === 'send_message').length,
      };
    })).toEqual({ files: 1, sends: 0 });
  });
}

export function registerAHeldVoiceFailureStaysWithItsCapturedAccountAndConversation() {
  test('a held voice failure stays with its captured account and conversation', async ({ page }) => {
    let releaseUpload!: () => void;
    const released = new Promise<void>((resolve) => { releaseUpload = resolve; });
    let markStarted!: () => void;
    const started = new Promise<void>((resolve) => { markStarted = resolve; });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'mediaDevices', {
        configurable: true, value: {
          getUserMedia: async () => ({ getTracks: () => [{ stop: () => { } }] }),
        }
      });
      class FakeMediaRecorder {
        static isTypeSupported() { return true; }
        state: RecordingState = 'inactive';
        mimeType = 'audio/webm';
        ondataavailable: ((event: BlobEvent) => void) | null = null;
        onstop: (() => void) | null = null;
        start() { this.state = 'recording'; }
        stop() {
          this.state = 'inactive';
          this.ondataavailable?.({ data: new Blob(['voice'], { type: this.mimeType }) } as BlobEvent);
          this.onstop?.();
        }
      }
      Object.defineProperty(window, 'MediaRecorder', { configurable: true, value: FakeMediaRecorder });
    });
    await page.route(/\/api\/uploads/, async (route) => {
      markStarted();
      await released;
      await route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'voice storage unavailable' }) });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.chatStore.setState({
        nickname: 'alice', activeAccountId: 'account-a', accountGeneration: 1,
        operationGeneration: 'generation-a', ws: { send: () => true }
      });
      scope.uiStore.getState().setActiveServer('server-1');
      scope.uiStore.getState().setActiveChannel('general');
    });
    await page.getByTitle('Record voice message').click();
    await page.getByTitle('Stop and send').click();
    await started;
    await page.evaluate(() => (window as typeof window & { chatStore: ChatStore }).chatStore.setState({
      nickname: 'bob', activeAccountId: 'account-b', accountGeneration: 2, operationGeneration: 'generation-b', ws: { send: () => true },
    }));
    releaseUpload();
    await page.waitForTimeout(100);
    await expect(page.getByRole('alert')).toHaveCount(0);
    await page.evaluate(() => (window as typeof window & { chatStore: ChatStore }).chatStore.setState({
      nickname: 'alice', activeAccountId: 'account-a', accountGeneration: 1, operationGeneration: 'generation-a', ws: { send: () => true },
    }));
    await expect(page.getByRole('alert')).toContainText('voice storage unavailable');
    await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  });
}

export function registerReplyTargetsRemainScopedWhileNavigatingBetweenConversations() {
  test('reply targets remain scoped while navigating between conversations', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.chatStore.setState({
        messages: {
          'server:alpha': [{ id: 'alpha-message', from: 'alice', content: 'alpha reply', timestamp: '2026-09-06T00:00:00Z' }],
          'server:beta': [{ id: 'beta-message', from: 'bob', content: 'beta reply', timestamp: '2026-09-06T00:01:00Z' }],
        }
      });
      scope.uiStore.getState().setActiveServer('server');
      scope.uiStore.getState().setActiveChannel('alpha');
    });
    await page.locator('[data-message-id="alpha-message"]').hover();
    await page.getByTitle('Reply').click();
    await expect(page.getByText('alpha reply', { exact: true })).toHaveCount(2);
    await page.evaluate(() => (window as typeof window & { uiStore: UiStore }).uiStore.getState().setActiveChannel('beta'));
    await expect(page.getByText('Replying to')).toHaveCount(0);
    await page.locator('[data-message-id="beta-message"]').hover();
    await page.getByTitle('Reply').click();
    await expect(page.getByText('beta reply', { exact: true })).toHaveCount(2);
    await page.evaluate(() => (window as typeof window & { uiStore: UiStore }).uiStore.getState().setActiveChannel('alpha'));
    await expect(page.getByText('Replying to')).toBeVisible();
    await expect(page.getByText('alpha reply', { exact: true })).toHaveCount(2);
  });
}

export function registerFailedComposerUploadStaysAttachedAndCanBeRetriedVisibly() {
  test('failed composer upload stays attached and can be retried visibly', async ({ page }) => {
    let attempts = 0;
    await page.route(/\/api\/uploads/, async (route) => {
      attempts += 1;
      if (attempts === 1) {
        await route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Media storage is temporarily unavailable' }) });
        return;
      }
      await route.fulfill({
        status: 201, contentType: 'application/json', body: JSON.stringify({
          id: 'attachment-retried', filename: 'retry.txt', content_type: 'text/plain', file_size: 5,
          url: '/api/uploads/attachment-retried',
        })
      });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sentCommands: unknown[] };
      stores.sentCommands = [];
      stores.chatStore.setState({
        nickname: 'alice', activeAccountId: 'account-a', accountGeneration: 1,
        operationGeneration: 'generation-a', ws: { send: (command: unknown) => { stores.sentCommands.push(command); return true; } },
      });
      stores.uiStore.getState().setActiveServer('server-1');
      stores.uiStore.getState().setActiveChannel('general');
    });
    const input = page.getByPlaceholder('Message general');
    await input.fill('retry this attachment');
    await page.locator('input[type=file]').setInputFiles({
      name: 'retry.txt', mimeType: 'text/plain', buffer: Buffer.from('retry'),
    });
    await input.press('Enter');
    await expect(page.getByRole('alert')).toContainText('1 of 1 uploads failed');
    await expect(page.getByText('retry.txt')).toBeVisible();
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => {
      const scope = window as typeof window & { sentCommands: Array<{ type?: string; attachment_ids?: string[] }> };
      return scope.sentCommands.find((command) => command.type === 'send_message')?.attachment_ids?.[0];
    })).toBe('attachment-retried');
  });
}
