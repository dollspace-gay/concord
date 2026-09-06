import { expect, test, type WebSocketRoute } from '@playwright/test';

type ChatStore = typeof import('../../src/stores/chatStore').useChatStore;
type UiStore = typeof import('../../src/stores/uiStore').useUiStore;

test('direct conversations recover offline history and route live messages by canonical conversation', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sent: unknown[] = [];
    store.setState({
      nickname: 'carmilla', activeAccountId: 'self', accountGeneration: 2,
      operationGeneration: 'generation', ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
    });
    store.getState().handleEvent({
      type: 'direct_conversation_list', conversations: [{
        id: 'dm-1', peer_id: 'peer-id', peer_username: 'laurelai', peer_avatar_url: null,
        last_message_at: '2026-09-05T01:00:00Z', unread_count: 1,
      }],
    });
    const syncRequest = sent.find((entry) => (entry as { type?: string }).type === 'sync') as { request_id: string };
    store.getState().handleEvent({
      type: 'sync_snapshot', request_id: syncRequest.request_id, snapshot: {
        cursor: 'cursor', operation_generation: 'generation', protocol_version: 2,
        history_before: {}, reactions: [], read_states: [], messages: [{
          attachments: [], content: 'delivered while offline', content_format: 'plain',
          conversation_id: 'dm-1', created_at: '2026-09-05T01:00:00Z', deleted: false,
          edited_at: null, entity_version: 1, mentions: [], message_id: 'offline-1', reply_to: null,
          reply_to_id: null, sender_id: 'peer-id', sender_nick: 'laurelai', sequence: '1',
        }],
      },
    });
    store.getState().handleEvent({
      type: 'message', id: 'live-1', conversation_id: 'dm-1', from: 'laurelai', target: 'carmilla',
      content: 'live', timestamp: '2026-09-05T01:01:00Z', server_id: null,
    });
    const liveProjection = {
      conversation_id: 'dm-1', descriptor: {}, entity_id: 'live-1', entity_type: 'message',
      entity_version: 1, kind: 'message_created', reaction: null, read_state: null,
      message: {
        attachments: [], content: 'live', content_format: 'plain', conversation_id: 'dm-1',
        created_at: '2026-09-05T01:01:00Z', deleted: false, edited_at: null, entity_version: 1,
        mentions: [], message_id: 'live-1', reply_to: null, reply_to_id: null,
        sender_id: 'peer-id', sender_nick: 'laurelai', sequence: '2',
      },
    } as const;
    store.getState().handleEvent({ type: 'durable_event', event: liveProjection });
    store.getState().handleEvent({
      type: 'durable_event', event: {
        ...liveProjection, entity_version: 2, kind: 'message_deleted',
        message: { ...liveProjection.message, entity_version: 2, deleted: true, content: null },
      },
    });
    store.getState().handleEvent({
      type: 'message', id: 'live-1', conversation_id: 'dm-1', from: 'laurelai', target: 'carmilla',
      content: 'stale live copy', timestamp: '2026-09-05T01:01:00Z', server_id: null,
    });
    store.getState().handleEvent({
      type: 'message', id: 'offline-1', conversation_id: 'dm-1', from: 'laurelai', target: 'carmilla',
      content: 'duplicate snapshot copy', timestamp: '2026-09-05T01:00:00Z', server_id: null,
    });
    const accepted = store.getState().sendDirectMessage('dm-1', 'laurelai', 'reply');
    const command = sent.find((entry) => (entry as { type?: string }).type === 'send_direct_message') as { nonce: string };
    store.getState().handleEvent({
      type: 'message_ack', id: 'reply-1', server_id: '', channel: 'laurelai', conversation_id: 'dm-1',
      request_id: command.nonce, client_message_id: command.nonce, sequence: '3',
      persisted_at: '2026-09-05T01:02:00Z', replayed: false, nonce: command.nonce,
    });
    return {
      accepted,
      contents: store.getState().messages['dm:dm-1'].map((message) => message.content),
      ids: store.getState().messages['dm:dm-1'].map((message) => message.id),
      command: sent.find((entry) => (entry as { type?: string }).type === 'send_direct_message'),
    };
  });
  expect(result.accepted).toBe(true);
  expect(result.contents).toEqual(['delivered while offline', '', 'reply']);
  expect(result.ids).toEqual(['offline-1', 'live-1', 'reply-1']);
  expect(result.command).toMatchObject({ type: 'send_direct_message', recipient: 'laurelai', content: 'reply' });
});

test('facade subscribers observe coordinated account and sync entity commits atomically', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const observed = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    store.setState({ activeAccountId: 'account-a', accountGeneration: 1, syncCursor: 'old',
      messages: { old: [{ id: 'old', from: 'old', content: 'old', timestamp: '2026-01-01T00:00:00Z' }] } });
    const snapshots: Array<{ account: string | null; generation: number; cursor: string | null; keys: string[] }> = [];
    const unsubscribe = store.subscribe((state) => snapshots.push({
      account: state.activeAccountId, generation: state.accountGeneration,
      cursor: state.syncCursor, keys: Object.keys(state.messages),
    }));
    store.setState({ activeAccountId: 'account-b', accountGeneration: 2, syncCursor: 'new', messages: { fresh: [] } });
    unsubscribe();
    return snapshots;
  });
  expect(observed).toEqual([{ account: 'account-b', generation: 2, cursor: 'new', keys: ['fresh'] }]);
});

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
      ws: { send: (command: unknown) => {
        sent.push(structuredClone(command) as typeof sent[number]);
        return true;
      } },
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
      messages: { 'server:#general': [{
        id: 'response-message', from: 'helper-bot', sender_id: 'bot', content: 'Choose an action',
        timestamp: '2026-09-06T12:00:00Z',
        rich_embeds: [{
          title: 'Verified result', description: '**Ready**', color: '#228855',
          url: 'https://example.test/result', image_url: 'https://images.example.test/result.gif',
          thumbnail_url: 'https://user:secret@images.example.test/private.gif',
          fields: [{ name: 'Status', value: 'Complete', inline: true }],
        }],
        components: [{ type: 'action_row', components: [
          { type: 'button', custom_id: 'confirm', label: 'Confirm', style: 'success' },
          { type: 'select_menu', custom_id: 'priority', placeholder: 'Priority', options: [
            { label: 'High', value: 'high' }, { label: 'Low', value: 'low' },
          ] },
        ] }],
      }] },
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

test('server and profile images remain opt-in before third-party requests', async ({ page }) => {
  let imageRequests = 0;
  await page.route('https://profiles.example.test/**', async (route) => {
    imageRequests += 1;
    await route.fulfill({ status: 200, contentType: 'image/gif', body: Buffer.from('R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==', 'base64') });
  });
  await page.goto('/layout-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    scope.chatStore.setState({
      servers: [{ id: 'server', name: 'Garden', owner_id: 'owner', created_at: '2026-01-01T00:00:00Z', icon_url: 'https://profiles.example.test/server.gif' }],
      userProfiles: { member: { id: 'member', username: 'Member', avatar_url: 'https://profiles.example.test/avatar.gif', banner_url: 'https://profiles.example.test/banner.gif', created_at: '2026-01-01T00:00:00Z' } },
    });
    scope.uiStore.getState().setShowUserProfile('member');
  });
  await expect(page.getByRole('button', { name: 'Load external image: Garden icon' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Load external image: profile avatar' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Load external image: profile banner' })).toBeVisible();
  expect(imageRequests).toBe(0);
  await page.getByRole('button', { name: 'Load external image: profile avatar' }).click();
  await expect.poll(() => imageRequests).toBe(1);
});

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
      members: { 'server:#general': [
        { nickname: 'Local member', user_id: 'local', server_avatar_url: '/api/uploads/20000000-0000-4000-8000-000000000002' },
        { nickname: 'Relative path', user_id: 'relative', avatar_url: '/admin' },
        { nickname: 'Protocol relative', user_id: 'protocol-relative', avatar_url: '//images.example.test/avatar.gif' },
        { nickname: 'Private host', user_id: 'private-host', avatar_url: 'https://127.0.0.1/avatar.gif' },
        { nickname: 'Private scheme', user_id: 'private-scheme', avatar_url: 'javascript:alert(1)' },
        { nickname: 'Malformed upload', user_id: 'malformed-upload', avatar_url: '/api/uploads/not-a-uuid' },
      ] },
      userProfiles: { local: {
        id: 'local', username: 'Local member', created_at: '2026-01-01T00:00:00Z',
        avatar_url: '/api/uploads/30000000-0000-4000-8000-000000000003',
        banner_url: '/api/uploads/40000000-0000-4000-8000-000000000004',
      } },
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

test('slash command autocomplete is keyboard accessible and submits typed arguments', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const ui = (window as typeof window & { uiStore: UiStore }).uiStore;
    const sent: unknown[] = [];
    (window as typeof window & { slashCommandsSent?: unknown[] }).slashCommandsSent = sent;
    ui.setState({ activeServer: 'server', activeChannel: '#general', activeDirectConversation: null });
    store.setState({
      nickname: 'laurelai', activeAccountId: 'user', connected: true,
      ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
      slashCommands: { server: [{
        id: 'hello-command', bot_user_id: 'bot', server_id: 'server', name: 'hello',
        description: 'Greet a member', created_at: '2026-09-06T12:00:00Z',
        options: [{ name: 'member', description: 'Member name', option_type: 'string', required: true }],
      }] },
    });
  });
  const composer = page.getByRole('textbox', { name: /Message/ });
  await composer.fill('/he');
  await expect(page.getByRole('listbox', { name: 'Slash commands' })).toBeVisible();
  await expect(page.getByRole('option', { name: /hello/ })).toHaveAttribute('aria-selected', 'true');
  await composer.press('Tab');
  await expect(composer).toHaveValue('/hello ');
  await composer.fill('/hello Laurelai');
  await composer.press('Enter');
  const command = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const command = (window as typeof window & { slashCommandsSent: Array<{ type?: string; request_id?: string }> }).slashCommandsSent.find((entry) => entry.type === 'invoke_slash_command')!;
    store.getState().handleEvent({ type: 'interaction_invoked', request_id: command.request_id! });
    return command;
  });
  expect(command).toMatchObject({
    type: 'invoke_slash_command', server_id: 'server', channel: '#general', command_name: 'hello',
    args_json: '{"member":"Laurelai"}',
  });
  await expect(composer).toHaveValue('');
});

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
      slashCommands: { server: [{
        id: 'fail-command', bot_user_id: 'bot', name: 'fail', description: 'Reject this command',
        options: [], created_at: '2026-09-06T12:00:00Z',
      }] },
      messages: { 'server:#general': [{
        id: 'component-message', from: 'helper-bot', content: '', timestamp: '2026-09-06T12:00:00Z',
        components: [{ type: 'action_row', components: [
          { type: 'button', custom_id: 'denied', label: 'Denied action', style: 'danger' },
        ] }],
      }] },
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

test('removed conversations prune protected state and ignore delayed obsolete snapshots', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const state = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sent: Array<{ type?: string; request_id?: string }> = [];
    const channel = {
      id: 'removed-channel', conversation_id: 'removed-conversation', server_id: 'server-1',
      name: 'private', topic: '', member_count: 1, category_id: null, position: 0,
      is_private: true, channel_type: 'text', thread_parent_message_id: null,
      archived: false, slowmode_seconds: 0, is_nsfw: false,
    };
    store.setState({
      nickname: 'carmilla', ws: { send: (value: unknown) => { sent.push(structuredClone(value) as typeof sent[number]); return true; } },
    });
    store.getState().handleEvent({ type: 'channel_list', server_id: 'server-1', channels: [channel] });
    const obsolete = sent.find((entry) => entry.type === 'sync')!.request_id!;
    store.setState({
      messages: { 'server-1:private': [{ id: 'secret', from: 'alice', content: 'secret', timestamp: '2026-01-01T00:00:00Z' }] },
      members: { 'server-1:private': [{ nickname: 'alice' }] }, unreadCounts: { 'server-1:private': 1 },
      hasMore: { 'server-1:private': true }, entityVersions: { 'message:secret': 1 },
    });
    store.getState().handleEvent({ type: 'channel_list', server_id: 'server-1', channels: [] });
    store.getState().handleEvent({
      type: 'sync_snapshot', request_id: obsolete, snapshot: {
        cursor: 'obsolete', operation_generation: 'old', protocol_version: 2, history_before: {}, reactions: [], read_states: [],
        messages: [{ attachments: [], content: 'secret', content_format: 'plain', conversation_id: 'removed-conversation',
          created_at: '2026-01-01T00:00:00Z', deleted: false, edited_at: null, entity_version: 1, mentions: [],
          message_id: 'secret', reply_to: null, reply_to_id: null, sender_id: 'alice', sender_nick: 'alice', sequence: '1' }],
      },
    });
    const next = store.getState();
    return { messages: next.messages, members: next.members, unread: next.unreadCounts, versions: next.entityVersions, cursor: next.syncCursor };
  });
  expect(state).toEqual({ messages: {}, members: {}, unread: {}, versions: {}, cursor: null });
});

test('a rejected direct send removes its optimistic row and remains retryable with full context', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sent: unknown[] = [];
    store.setState({
      nickname: 'carmilla', activeAccountId: 'self', accountGeneration: 3, operationGeneration: 'generation',
      directConversations: [{ id: 'dm-1', peer_id: 'peer', peer_username: 'laurelai', last_message_at: null, unread_count: 0 }],
      ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
      replyingTo: { id: 'prior', from: 'laurelai', content_preview: 'prior' },
    });
    store.getState().sendDirectMessage('dm-1', 'laurelai', 'answer', [{
      id: 'attachment', filename: 'answer.txt', content_type: 'text/plain', file_size: 6, url: '/api/uploads/attachment',
    }]);
    const command = sent[0] as { request_id: string };
    store.getState().handleEvent({
      type: 'command_error', request_id: command.request_id, code: 'FORBIDDEN', message: 'blocked', retryable: false,
    });
    const failed = store.getState().failedCompositions[0];
    return {
      messages: store.getState().messages['dm:dm-1'],
      failed: failed && { content: failed.content, attachment: failed.attachments[0]?.id, reply: failed.replyTo?.id,
        conversationId: failed.conversationId, recipient: failed.recipient },
    };
  });
  expect(result.messages).toEqual([]);
  expect(result.failed).toEqual({ content: 'answer', attachment: 'attachment', reply: 'prior', conversationId: 'dm-1', recipient: 'laurelai' });
});

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

test('formatted messages expose safe links and keyboard-operable spoilers', async ({ page }) => {
  await page.goto('/formatted-harness.html');
  await expect(page.locator('strong')).toHaveText('bold');
  await expect(page.locator('em')).toHaveText('italic');
  await expect(page.locator('del')).toHaveText('removed');
  await expect(page.locator('blockquote')).toHaveText('quoted');
  await expect(page.locator('pre code')).toHaveCount(0);
  const inlineCode = page.locator('code');
  await expect(inlineCode).toHaveText('code');
  const safeLink = page.getByRole('link', { name: 'https://example.com/path' });
  await expect(safeLink).toHaveAttribute('href', 'https://example.com/path');
  await expect(safeLink).toHaveAttribute('rel', 'noreferrer');
  await expect(page.locator('#formatted-message script')).toHaveCount(0);
  await expect(page.locator('body')).toContainText('<script>alert(1)</script>');
  const spoiler = page.getByRole('button', { name: 'Reveal spoiler' });
  await spoiler.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Hide spoiler' })).toHaveAttribute('aria-expanded', 'true');
});

test('image viewer traps dismissal and restores focus to its opener', async ({ page }) => {
  await page.route('**/api/uploads/image', (route) => route.fulfill({
    status: 200, contentType: 'image/gif', body: Buffer.from('R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==', 'base64'),
  }));
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    scope.uiStore.setState({ activeServer: 'server', activeChannel: 'general', activeDirectConversation: null });
    scope.chatStore.setState({ messages: { 'server:general': [{
      id: 'image-message', from: 'alice', content: '', timestamp: '2026-09-06T00:00:00Z',
      attachments: [{ id: 'image', filename: 'proof.gif', content_type: 'image/gif', file_size: 1, url: '/api/uploads/image' }],
    }] } });
  });
  const opener = page.getByRole('button').filter({ has: page.getByAltText('proof.gif') });
  await opener.focus();
  await opener.click();
  await expect(page.getByRole('dialog', { name: 'Image viewer: proof.gif' })).toBeVisible();
  await expect(page.getByTitle('Open original')).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByTitle('Zoom in')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test('nested dialogs skip hidden controls and Escape closes only the top dialog', async ({ page }) => {
  await page.goto('/dialog-harness.html');
  const outerOpener = page.getByRole('button', { name: 'Open outer' });
  await outerOpener.focus();
  await outerOpener.click();
  const innerOpener = page.getByRole('button', { name: 'Open inner' });
  await expect(innerOpener).toBeFocused();
  await innerOpener.click();
  await expect(page.getByRole('button', { name: 'Inner first' })).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByRole('button', { name: 'Inner last' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Inner' })).toHaveCount(0);
  await expect(page.getByRole('dialog', { name: 'Outer' })).toBeVisible();
  await expect(innerOpener).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(outerOpener).toBeFocused();
});

test('workspace settings dialogs open and close by keyboard with focus restoration', async ({ page }) => {
  await page.goto('/layout-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    scope.chatStore.setState({ servers: [{ id: 'server', name: 'Garden', owner_id: 'alice', created_at: '2026-01-01T00:00:00Z', my_permissions: 8 }],
      channels: { server: [{ id: 'general', conversation_id: 'conversation', server_id: 'server', name: 'general', topic: '',
        member_count: 1, category_id: null, position: 0, is_private: false, channel_type: 'text',
        thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false }] }, activeAccountId: 'alice' });
    scope.uiStore.getState().setActiveServer('server');
  });
  const userSettings = page.getByRole('button', { name: /Settings$/ }).filter({ hasText: 'Settings' });
  await userSettings.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close settings' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(userSettings).toBeFocused();

  const serverSettings = page.getByTitle('Server Settings');
  await serverSettings.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Garden Settings' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(serverSettings).toBeFocused();

  for (const name of ['Community', 'Moderation']) {
    const opener = page.getByTitle(name);
    await opener.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog', { name })).toBeVisible();
    await expect(page.getByRole('button', { name: `Close ${name.toLowerCase()}` })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog', { name })).toHaveCount(0);
    await expect(opener).toBeFocused();
  }
});

test('current rules gate preserves a disconnected acceptance and retries after reconnect', async ({ page }) => {
  await page.goto('/layout-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    scope.chatStore.setState({
      connected: false,
      activeAccountId: 'member',
      servers: [{ id: 'server', name: 'Garden', owner_id: 'owner', created_at: '2026-01-01T00:00:00Z' }],
      communitySettings: { server: { server_id: 'server', is_discoverable: false, rules_text: 'Be kind.', rules_accepted: false } },
    });
    scope.uiStore.getState().setActiveServer('server');
  });

  const dialog = page.getByRole('dialog', { name: 'Accept server rules' });
  await expect(dialog).toContainText('Be kind.');
  await dialog.getByRole('button', { name: 'Accept rules' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Not connected');
  await expect(dialog.getByRole('button', { name: 'Accept rules' })).toBeEnabled();

  const requestId = await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
    scope.chatStore.setState({ connected: true, ws: { send: (value: unknown) => { scope.rulesCommand = structuredClone(value) as { request_id: string }; return true; } } });
    return null;
  });
  expect(requestId).toBeNull();
  await dialog.getByRole('button', { name: 'Accept rules' }).click();
  await page.waitForFunction(() => Boolean((window as typeof window & { rulesCommand?: unknown }).rulesCommand));
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
    scope.chatStore.getState().handleEvent({ type: 'lifecycle_command_succeeded', request_id: scope.rulesCommand!.request_id });
  });
  await expect(dialog).toHaveCount(0);

  await page.getByTitle('Community').click();
  const community = page.getByRole('dialog', { name: 'Community' });
  await community.getByRole('button', { name: 'Create Invite' }).click();
  const maxUses = community.getByLabel('Max Uses (0 = unlimited)');
  await maxUses.fill('5');
  await community.getByRole('button', { name: 'Generate Invite' }).click();
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
    scope.chatStore.getState().handleEvent({ type: 'command_error', request_id: scope.rulesCommand!.request_id, code: 'FORBIDDEN', message: 'Invite denied', retryable: false });
  });
  await expect(community.getByRole('alert')).toContainText('Invite denied');
  await expect(maxUses).toHaveValue('5');
  await community.getByRole('button', { name: 'Generate Invite' }).click();
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
    scope.chatStore.getState().handleEvent({ type: 'lifecycle_command_succeeded', request_id: scope.rulesCommand!.request_id });
  });
  await expect(maxUses).toHaveCount(0);
});

test('mobile chat uses the full viewport and provides a conversation back action', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 667 });
  await page.goto('/layout-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    stores.chatStore.setState({
      nickname: 'carmilla', activeAccountId: 'self', operationGeneration: 'generation',
      channels: { 'server-1': [{ id: 'channel', conversation_id: 'conversation', server_id: 'server-1', name: 'general',
        topic: '', member_count: 1, position: 0, is_private: false, channel_type: 'text', archived: false,
        slowmode_seconds: 0, is_nsfw: false }] },
    });
    stores.uiStore.getState().setActiveServer('server-1');
    stores.uiStore.getState().setActiveChannel('general');
  });
  await expect(page.getByRole('button', { name: 'Back to conversations' })).toBeVisible();
  const composer = page.getByPlaceholder('Message general');
  await expect(composer).toBeVisible();
  const box = await composer.boundingBox();
  expect(box!.width).toBeGreaterThan(200);
  await page.getByRole('button', { name: 'Back to conversations' }).click();
  await expect(page.getByText('Concord')).toBeVisible();
});

test('workspace remains usable without horizontal overflow at tablet and desktop widths', async ({ page }) => {
  for (const width of [768, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/layout-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.chatStore.setState({ nickname: 'alice', activeAccountId: 'alice', channels: { server: [{
        id: 'general', conversation_id: 'conversation', server_id: 'server', name: 'general', topic: '',
        member_count: 1, category_id: null, position: 0, is_private: false, channel_type: 'text',
        thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
      }] } });
      scope.uiStore.getState().setActiveServer('server');
      scope.uiStore.getState().setActiveChannel('general');
    });
    await expect(page.getByPlaceholder('Message general')).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  }
});

test('quick switcher is keyboard navigable and restores focus after selection', async ({ page }) => {
  await page.goto('/layout-harness.html');
  await page.evaluate(() => {
    const typed = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    typed.chatStore.setState({
      servers: [{ id: 'server-1', name: 'Night School', member_count: 1, role: 'owner', my_permissions: 0 }],
      channels: { 'server-1': [{
        id: 'channel-1', conversation_id: 'conversation-1', server_id: 'server-1', name: '#general',
        topic: '', member_count: 1, category_id: null, position: 0, is_private: false,
        channel_type: 'text', thread_parent_message_id: null, archived: false,
        slowmode_seconds: 0, is_nsfw: false,
      }] },
      ws: { send: () => true },
    });
  });
  const directMessages = page.getByRole('button', { name: 'Direct messages' });
  await directMessages.focus();
  await page.keyboard.press('Control+k');
  const dialog = page.getByRole('dialog', { name: 'Quick switcher' });
  await expect(dialog).toBeVisible();
  const search = page.getByRole('textbox', { name: 'Search servers and channels' });
  await expect(search).toBeFocused();
  await search.fill('general');
  const channelResult = page.getByRole('button', { name: /general.*Night School/ });
  await channelResult.focus();
  await page.keyboard.press('Enter');
  await expect(dialog).toBeHidden();
  await expect(directMessages).toBeFocused();
  const selected = await page.evaluate(() => {
    const typed = window as typeof window & { uiStore: UiStore };
    return { server: typed.uiStore.getState().activeServer, channel: typed.uiStore.getState().activeChannel };
  });
  expect(selected).toEqual({ server: 'server-1', channel: '#general' });
});

test('snapshot and replay rebuild normalized conversation state', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);

  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sent: unknown[] = [];
    store.setState({ nickname: 'carmilla', ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } } });
    store.getState().handleEvent({
      type: 'channel_list',
      server_id: 'server-1',
      channels: [{
        id: 'channel-1', conversation_id: 'channel:6368616E6E656C2D31', server_id: 'server-1',
        name: 'general', topic: '', member_count: 2, category_id: null, position: 0,
        is_private: false, channel_type: 'text', thread_parent_message_id: null,
        archived: false, slowmode_seconds: 0, is_nsfw: false,
      }],
    });
    const baseMessage = {
      attachments: [], content: 'before', content_format: 'plain',
      conversation_id: 'channel:6368616E6E656C2D31', created_at: '2026-01-01T00:00:00Z',
      deleted: false, edited_at: null, entity_version: 1, mentions: [], message_id: 'm1',
      reply_to: null, reply_to_id: null, sender_id: 'user-2', sender_nick: 'alice', sequence: '2',
    };
    const snapshotRequest = sent.find((entry) => (entry as { type?: string }).type === 'sync') as { request_id: string };
    store.getState().handleEvent({
      type: 'sync_snapshot', request_id: snapshotRequest.request_id, snapshot: {
        cursor: 'cursor-1', operation_generation: 'generation-2', protocol_version: 2,
        history_before: { 'channel:6368616E6E656C2D31': 'm0' }, messages: [baseMessage],
        reactions: [{ message_id: 'm1', emoji: 'heart', count: 3, reacted_by_me: true }],
        read_states: [{ conversation_id: 'channel:6368616E6E656C2D31', entity_version: 1, message_id: 'm0', sequence: '1' }],
      },
    });
    store.getState().handleEvent({
      type: 'channel_list', server_id: 'server-1', channels: [...store.getState().channels['server-1'], {
        id: 'channel-2', conversation_id: 'channel:6368616E6E656C2D32', server_id: 'server-1',
        name: 'other', topic: '', member_count: 2, category_id: null, position: 1,
        is_private: false, channel_type: 'text', thread_parent_message_id: null,
        archived: false, slowmode_seconds: 0, is_nsfw: false,
      }],
    });
    const replayRequest = [...sent].reverse().find((entry) => (entry as { type?: string }).type === 'sync') as { request_id: string };
    store.getState().handleEvent({
      type: 'replay_batch', request_id: replayRequest.request_id, batch: {
        cursor: 'cursor-2', operation_generation: 'generation-2', protocol_version: 2, has_more: false,
        events: [{
          conversation_id: 'channel:6368616E6E656C2D31', descriptor: {}, entity_id: 'm1',
          entity_type: 'message', entity_version: 2, kind: 'message_updated',
          message: { ...baseMessage, content: 'after', edited_at: '2026-01-01T00:01:00Z', entity_version: 2 },
          reaction: null, read_state: null,
        }],
      },
    });
    const state = store.getState();
    return {
      cursor: state.syncCursor,
      resumeCursor: (sent.findLast((entry) => (entry as { type?: string }).type === 'sync') as { cursor?: string }).cursor,
      generation: state.operationGeneration,
      content: state.messages['server-1:general'][0].content,
      reaction: state.messages['server-1:general'][0].reactions[0],
      unread: state.unreadCounts['server-1:general'],
      hasMore: state.hasMore['server-1:general'],
    };
  });

  expect(result).toEqual({
    cursor: 'cursor-2', resumeCursor: undefined, generation: 'generation-2', content: 'after',
    reaction: { emoji: 'heart', count: 3, user_ids: ['__self__'] }, unread: 1, hasMore: true,
  });
});

test('resync clears protected projections before rebuilding', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const state = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    store.setState({
      servers: [{ id: 'server-1', name: 'Server', member_count: 1 }],
      channels: { 'server-1': [{ id: 'channel-1' }] }, messages: { stale: [{ id: 'm1' }] },
      members: { stale: [{ nickname: 'alice' }] }, roles: { 'server-1': [{ id: 'r1' }] },
      categories: { 'server-1': [{ id: 'c1' }] }, pinnedMessages: { stale: [{ id: 'm1' }] },
      threads: { stale: [{ id: 't1' }] }, bookmarks: [{ id: 'b1' }], unreadCounts: { stale: 2 },
      operationGeneration: 'old', syncCursor: 'old',
      drafts: { 'server-1:general': 'unsent draft' },
    });
    store.getState().handleEvent({ type: 'resync_required', reason: 'cursor expired' });
    const next = store.getState();
    return {
      servers: next.servers, channels: next.channels, messages: next.messages, members: next.members,
      roles: next.roles, categories: next.categories, pins: next.pinnedMessages,
      threads: next.threads, bookmarks: next.bookmarks, unread: next.unreadCounts,
      generation: next.operationGeneration, cursor: next.syncCursor,
      drafts: next.drafts,
    };
  });
  expect(state).toEqual({
    servers: [], channels: {}, messages: {}, members: {}, roles: {}, categories: {}, pins: {},
    threads: {}, bookmarks: [], unread: {}, generation: null, cursor: null,
    drafts: { 'server-1:general': 'unsent draft' },
  });
});

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

test('account change cancels a delayed private-command retry', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const sentByB = await page.evaluate(async () => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sentA: unknown[] = [];
    const sentB: unknown[] = [];
    store.setState({
      nickname: 'account-a', accountGeneration: 4, operationGeneration: 'generation-a',
      ws: { send: (command: unknown) => { sentA.push(command); return true; }, disconnect: () => {} },
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

test('single and bulk deletes redact replies and evict loaded search pin and bookmark copies', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const message = (id: string, replyId?: string) => ({
      id, from: 'alice', sender_id: 'alice', sequence: id === 'first' ? '1' : '2', content: id,
      timestamp: '2026-09-06T00:00:00Z', attachments: [],
      reply_to: replyId ? { id: replyId, from: 'alice', content_preview: `preview-${replyId}` } : null,
    });
    const pin = (id: string) => ({ id: `pin-${id}`, message_id: id, channel_id: 'channel', pinned_by: 'alice',
      pinned_at: '2026-09-06T00:00:00Z', from: 'alice', content: id, timestamp: '2026-09-06T00:00:00Z' });
    const bookmark = (id: string) => ({ id: `bookmark-${id}`, message_id: id, channel_id: 'channel', from: 'alice',
      content: id, timestamp: '2026-09-06T00:00:00Z', created_at: '2026-09-06T00:00:00Z' });
    const search = (id: string) => ({ id, from: 'alice', content: id, timestamp: '2026-09-06T00:00:00Z',
      channel_id: 'channel', channel_name: '#general' });
    store.setState({
      durableMode: false,
      messages: { 'server:#general': [message('first'), message('reply-first', 'first'), message('second'), message('reply-second', 'second')] },
      pinnedMessages: { 'server:#general': [pin('first'), pin('second')] },
      bookmarks: [bookmark('first'), bookmark('second')],
      searchResults: [search('first'), search('second')],
    });
    store.getState().handleEvent({ type: 'message_delete', id: 'first', server_id: 'server', channel: '#general' });
    store.getState().handleEvent({ type: 'bulk_message_delete', message_ids: ['second'], server_id: 'server', channel: '#general' });
    const state = store.getState();
    return {
      messages: state.messages['server:#general'].map((entry) => ({ id: entry.id, preview: entry.reply_to?.content_preview })),
      pins: state.pinnedMessages['server:#general'], bookmarks: state.bookmarks, search: state.searchResults,
    };
  });
  expect(result).toEqual({
    messages: [{ id: 'reply-first', preview: '' }, { id: 'reply-second', preview: '' }],
    pins: [], bookmarks: [], search: [],
  });
});

test('a delayed read projection preserves messages with a newer canonical sequence', async ({ page }) => {
  await page.goto('/test-harness.html');
  const result = await page.evaluate(async () => {
    const { useChatStore: store } = await import('/src/stores/chatStore.ts');
    store.setState({
      activeAccountId: 'self-id',
      nickname: 'same-nickname',
      directConversations: [{
        id: 'opaque-direct', peer_id: 'peer-id', peer_username: 'same-nickname',
        last_message_at: null, unread_count: 0,
      }],
      messages: {}, unreadCounts: {}, entityVersions: {}, readSequences: {},
    });
    const event = (messageId: string, sequence: string) => ({
      conversation_id: 'opaque-direct', descriptor: {}, entity_id: messageId,
      entity_type: 'message', entity_version: 1, kind: 'message_created',
      message: {
        attachments: [], content: messageId, content_format: 'plain',
        conversation_id: 'opaque-direct', created_at: `2026-01-01T00:00:0${sequence}Z`,
        deleted: false, edited_at: null, entity_version: 1, mentions: [], message_id: messageId,
        reply_to: null, reply_to_id: null, sender_id: 'peer-id', sender_nick: 'same-nickname', sequence,
      },
      reaction: null, read_state: null,
    });
    store.getState().handleEvent({ type: 'durable_event', event: event('m6', '6') });
    store.getState().handleEvent({ type: 'durable_event', event: {
      conversation_id: 'opaque-direct', descriptor: {}, entity_id: 'read:opaque-direct',
      entity_type: 'read_state', entity_version: 1, kind: 'read_advanced', message: null, reaction: null,
      read_state: { conversation_id: 'opaque-direct', entity_version: 1, message_id: 'm5', sequence: '5' },
    }});
    const afterDelayedRead = store.getState().unreadCounts['dm:opaque-direct'];
    store.getState().handleEvent({ type: 'durable_event', event: {
      conversation_id: 'opaque-direct', descriptor: {}, entity_id: 'read:opaque-direct',
      entity_type: 'read_state', entity_version: 2, kind: 'read_advanced', message: null, reaction: null,
      read_state: { conversation_id: 'opaque-direct', entity_version: 2, message_id: 'm6', sequence: '6' },
    }});
    return {
      afterDelayedRead,
      afterCurrentRead: store.getState().unreadCounts['dm:opaque-direct'],
      senderId: store.getState().messages['dm:opaque-direct'][0].sender_id,
      sequence: store.getState().messages['dm:opaque-direct'][0].sequence,
    };
  });
  expect(result).toEqual({
    afterDelayedRead: 1, afterCurrentRead: 0, senderId: 'peer-id', sequence: '6',
  });
});

test('durable thread state updates known projections and ignores unknown descriptors', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const conversation = 'channel:thread-1';
    const thread = {
      id: 'thread-1', name: 'topic', channel_type: 'public_thread', parent_message_id: null,
      archived: false, auto_archive_minutes: 1440, message_count: 0,
      created_at: '2026-09-01T00:00:00Z',
    };
    store.setState({
      channels: { server: [{
        id: 'thread-1', conversation_id: conversation, server_id: 'server', name: 'topic',
        topic: '', member_count: 1, category_id: null, position: 0, is_private: false,
        channel_type: 'public_thread', thread_parent_message_id: null, archived: false,
        slowmode_seconds: 0, is_nsfw: false,
      }] },
      threads: { 'server:parent': [thread] },
    });
    store.getState().handleEvent({ type: 'durable_event', event: {
      conversation_id: conversation, descriptor: { archived: true, reason: 'manual' },
      entity_id: 'thread-1', entity_type: 'thread_state', entity_version: 2,
      kind: 'thread_state_changed', message: null, reaction: null, read_state: null,
    } });
    store.getState().handleEvent({ type: 'durable_event', event: {
      conversation_id: conversation, descriptor: { thread_id: 'thread-1', tag_ids: ['tag-new'] },
      entity_id: 'thread-1', entity_type: 'thread_tags', entity_version: 5,
      kind: 'thread_tags_updated', message: null, reaction: null, read_state: null,
    } });
    store.getState().handleEvent({ type: 'durable_event', event: {
      conversation_id: conversation, descriptor: { thread_id: 'thread-1', tag_ids: ['tag-stale'] },
      entity_id: 'thread-1', entity_type: 'thread_tags', entity_version: 4,
      kind: 'thread_tags_updated', message: null, reaction: null, read_state: null,
    } });
    store.getState().handleEvent({ type: 'durable_event', event: {
      conversation_id: conversation, descriptor: { future: true }, entity_id: 'future-state',
      entity_type: 'future_type', entity_version: 9, kind: 'future_kind',
      message: null, reaction: null, read_state: null,
    } });
    const state = store.getState();
    return {
      channelArchived: state.channels.server[0].archived,
      threadArchived: state.threads['server:parent'][0].archived,
      threadVersion: state.entityVersions['thread_state:thread-1'],
      tagVersion: state.entityVersions['thread_tags:thread-1'],
      tagIds: state.threads['server:parent'][0].tag_ids,
      futureVersion: state.entityVersions['future_type:future-state'],
    };
  });
  expect(result).toEqual({
    channelArchived: true,
    threadArchived: true,
    threadVersion: 2,
    tagVersion: 5,
    tagIds: ['tag-new'],
    futureVersion: undefined,
  });
});

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

test('resync clears every protected cache through the shared reset', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const marker = { private: true };
    (store.setState as unknown as (state: Record<string, unknown>) => void)({
      servers: [marker], channels: { private: [marker] }, messages: { private: [marker] },
      members: { private: [marker] }, hasMore: { private: true }, avatars: { private: 'x' },
      typingUsers: { private: ['x'] }, replyingTo: marker, unreadCounts: { private: 1 },
      customEmoji: { private: marker }, roles: { private: [marker] }, categories: { private: [marker] },
      presences: { private: marker }, userProfiles: { private: marker }, searchResults: [marker],
      searchQuery: 'private', searchTotalCount: 1, pinnedMessages: { private: [marker] },
      threads: { private: [marker] }, forumTags: { private: [marker] }, bookmarks: [marker], notificationSettings: { private: [marker] },
      auditLog: { private: [marker] }, bans: { private: [marker] }, automodRules: { private: [marker] },
      invites: { private: [marker] }, serverEvents: { private: [marker] },
      communitySettings: { private: marker }, discoverableServers: [marker], templates: { private: [marker] },
      webhooks: { private: [marker] }, slashCommands: { private: [marker] }, botTokens: [marker],
      oauth2Apps: [marker], blueskyIdentities: { private: marker }, atprotoSyncEnabled: true,
      stickers: { private: [marker] }, allUserEmoji: [marker], serverAvatars: { private: marker },
      maxMessageLength: 1, maxFileSizeMb: 1,
    });
    store.getState().handleEvent({ type: 'resync_required', reason: 'authority changed' });
    const state = store.getState() as unknown as Record<string, unknown>;
    const emptyCollections = [
      'servers', 'channels', 'messages', 'members', 'hasMore', 'avatars', 'typingUsers',
      'unreadCounts', 'customEmoji', 'roles', 'categories', 'presences', 'userProfiles',
      'pinnedMessages', 'threads', 'forumTags', 'bookmarks', 'notificationSettings', 'auditLog', 'bans', 'automodRules',
      'invites', 'serverEvents', 'communitySettings', 'discoverableServers', 'templates',
      'webhooks', 'slashCommands', 'botTokens', 'oauth2Apps', 'blueskyIdentities', 'stickers',
      'allUserEmoji', 'serverAvatars',
    ];
    const leaks = emptyCollections.filter((key) => {
      const value = state[key];
      return Array.isArray(value) ? value.length !== 0 : Object.keys(value as object).length !== 0;
    });
    if (state.replyingTo !== null) leaks.push('replyingTo');
    if (state.searchResults !== null || state.searchQuery !== '' || state.searchTotalCount !== 0) leaks.push('search');
    if (state.atprotoSyncEnabled !== false) leaks.push('atprotoSyncEnabled');
    return { leaks, maxMessageLength: state.maxMessageLength, maxFileSizeMb: state.maxFileSizeMb };
  });
  expect(result).toEqual({ leaks: [], maxMessageLength: 4000, maxFileSizeMb: 100 });
});

test('a protected HTTP response held across resync cannot repopulate private state', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  let requested!: () => void;
  const seen = new Promise<void>((resolve) => { requested = resolve; });
  await page.route('**/api/servers/private/emoji', async (route) => {
    requested();
    await held;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([
      { id: 'private-emoji', server_id: 'private', name: 'secret', image_url: '/secret.png' },
    ]) });
  });
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    store.setState({ activeAccountId: 'alice' });
    (window as typeof window & { heldLoad?: Promise<void> }).heldLoad = store.getState().loadServerEmoji('private');
  });
  await seen;
  await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    store.getState().handleEvent({ type: 'resync_required', reason: 'permission changed' });
  });
  release();
  await page.evaluate(() => (window as typeof window & { heldLoad: Promise<void> }).heldLoad);
  expect(await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    return store.getState().customEmoji;
  })).toEqual({});
});

test('logout invalidates a held authentication check and reports revoke failure', async ({ page }) => {
  let releaseMe!: () => void;
  const heldMe = new Promise<void>((resolve) => { releaseMe = resolve; });
  let requestedMe!: () => void;
  const seenMe = new Promise<void>((resolve) => { requestedMe = resolve; });
  await page.route('**/api/auth/status', (route) => route.fulfill({
    status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: ['atproto'] }),
  }));
  await page.route('**/api/me', async (route) => {
    requestedMe();
    await heldMe;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ id: 'alice', username: 'alice' }) });
  });
  await page.route('**/api/auth/logout', (route) => route.fulfill({ status: 503, body: 'revoke unavailable' }));
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'authStore' in window);
  await page.evaluate(() => {
    const store = (window as typeof window & { authStore: { getState(): { checkAuth(): Promise<void> } } }).authStore;
    (window as typeof window & { heldCheck?: Promise<void> }).heldCheck = store.getState().checkAuth();
  });
  await seenMe;
  await page.evaluate(async () => {
    const store = (window as typeof window & { authStore: { getState(): { logout(): Promise<void> } } }).authStore;
    await store.getState().logout();
  });
  releaseMe();
  await page.evaluate(() => (window as typeof window & { heldCheck: Promise<void> }).heldCheck);
  expect(await page.evaluate(() => {
    const store = (window as typeof window & { authStore: { getState(): { user: unknown; error: string | null } } }).authStore;
    const { user, error } = store.getState();
    return { user, error };
  })).toEqual({ user: null, error: expect.stringContaining('server session could not be revoked') });
});

test('server folder saves serialize latest structure and retain failed edits for retry', async ({ page }) => {
  let releaseFirst!: () => void;
  const heldFirst = new Promise<void>((resolve) => { releaseFirst = resolve; });
  const payloads: unknown[] = [];
  let requests = 0;
  let failNext = false;
  await page.route('**/api/server-folders', async (route) => {
    if (route.request().method() !== 'PUT') return route.fulfill({ status: 200, body: '[]' });
    requests += 1;
    payloads.push(route.request().postDataJSON());
    if (requests === 1) await heldFirst;
    if (failNext) {
      failNext = false;
      return route.fulfill({ status: 503, body: 'folder store unavailable' });
    }
    return route.fulfill({ status: 204 });
  });
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'uiStore' in window);
  await page.evaluate(() => {
    const store = (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore;
    store.getState().hydrateServerFolders('alice');
    store.getState().addServerFolder('First', ['one']);
    store.getState().addServerFolder('Second', ['two']);
  });
  await expect.poll(() => requests).toBe(1);
  releaseFirst();
  await expect.poll(() => requests).toBe(2);
  await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore.getState().folderSyncStatus)).toBe('idle');
  expect(payloads[1]).toMatchObject([{ name: 'First' }, { name: 'Second' }]);

  failNext = true;
  await page.evaluate(() => {
    const store = (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore;
    store.getState().addServerFolder('Unsaved', ['three']);
  });
  await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore.getState().folderSyncStatus)).toBe('error');
  expect(await page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore.getState().serverFolders.map((folder) => folder.name))).toContain('Unsaved');
  await page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore.getState().retryServerFolderSync());
  await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore.getState().folderSyncStatus)).toBe('idle');
});

test('a held server folder load cannot overwrite a newer local edit or account', async ({ page }) => {
  let releaseAlice!: () => void;
  const heldAlice = new Promise<void>((resolve) => { releaseAlice = resolve; });
  let getCount = 0;
  await page.route('**/api/auth/status', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: [] }) }));
  await page.route('**/api/me', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ id: 'alice', username: 'alice' }) }));
  await page.route('**/api/server-folders', async (route) => {
    if (route.request().method() === 'PUT') return route.fulfill({ status: 204 });
    getCount += 1;
    if (getCount === 1) {
      await heldAlice;
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([{ id: 'old', name: 'Old', server_ids: ['one'] }]) });
    }
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([{ id: 'bob', name: 'Bob', server_ids: ['two'] }]) });
  });
  await page.routeWebSocket(/\/ws/, () => {});
  await page.goto('/app-harness.html');
  await page.waitForFunction(() => 'uiStore' in window);
  await expect.poll(() => getCount).toBe(1);
  await page.evaluate(() => {
    const scope = window as typeof window & {
      uiStore: typeof import('../../src/stores/uiStore').useUiStore;
      authStore: typeof import('../../src/stores/authStore').useAuthStore;
    };
    scope.uiStore.getState().addServerFolder('Local', ['local']);
    scope.authStore.setState({ user: { id: 'bob', username: 'bob' } });
  });
  await expect.poll(() => getCount).toBe(2);
  releaseAlice();
  await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../src/stores/uiStore').useUiStore }).uiStore.getState().serverFolders.map((folder) => folder.name))).toEqual(['Bob']);
});

test('held IRC token creation cannot reveal an old account credential after account switch', async ({ page }) => {
  let releaseCreate!: () => void;
  const heldCreate = new Promise<void>((resolve) => { releaseCreate = resolve; });
  let markCreateStarted!: () => void;
  const createStarted = new Promise<void>((resolve) => { markCreateStarted = resolve; });
  await page.route('**/api/tokens', async (route) => {
    if (route.request().method() === 'POST') {
      markCreateStarted();
      await heldCreate;
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
        id: 'old-token', token: 'irc-old-account-secret', label: 'old account', created_at: '2026-09-06T00:00:00Z',
      }) });
    } else {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '[]' });
    }
  });
  await page.goto('/settings-harness.html');
  await page.getByPlaceholder('Token label (optional)').fill('old account');
  await page.getByRole('button', { name: 'Generate' }).click();
  await createStarted;
  await page.evaluate(() => {
    const scope = window as typeof window & { authStore: typeof import('../../src/stores/authStore').useAuthStore; chatStore: ChatStore };
    scope.authStore.setState({ user: { id: 'account-b', username: 'bob' } });
    scope.chatStore.setState({ activeAccountId: 'account-b', protectedGeneration: 2 });
  });
  releaseCreate();
  await expect(page.getByText('irc-old-account-secret')).toHaveCount(0);
  await expect(page.getByPlaceholder('Token label (optional)')).toHaveValue('');
  await expect(page.getByRole('button', { name: 'Generate' })).toBeEnabled();
});

test('authentication bootstrap distinguishes signed out from a dependency failure', async ({ page }) => {
  let meStatus = 503;
  await page.route('**/api/auth/status', (route) => route.fulfill({
    status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: ['atproto'] }),
  }));
  await page.route('**/api/me', (route) => route.fulfill({ status: meStatus, body: meStatus === 401 ? 'unauthorized' : 'dependency unavailable' }));
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'authStore' in window);
  const dependencyFailure = await page.evaluate(async () => {
    const store = (window as typeof window & { authStore: typeof import('../../src/stores/authStore').useAuthStore }).authStore;
    store.setState({ user: { id: 'alice', username: 'alice' }, error: null });
    await store.getState().checkAuth();
    return { user: store.getState().user, error: store.getState().error };
  });
  expect(dependencyFailure).toEqual({
    user: { id: 'alice', username: 'alice' },
    error: expect.stringContaining('dependency unavailable'),
  });

  meStatus = 401;
  const signedOut = await page.evaluate(async () => {
    const store = (window as typeof window & { authStore: typeof import('../../src/stores/authStore').useAuthStore }).authStore;
    await store.getState().checkAuth();
    return { user: store.getState().user, error: store.getState().error };
  });
  expect(signedOut).toEqual({ user: null, error: null });
});

test('an authenticated app keeps its workspace and offers retry when sign-in verification is unavailable', async ({ page }) => {
  let meStatus = 200;
  await page.route('**/api/auth/status', (route) => route.fulfill({
    status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: ['atproto'] }),
  }));
  await page.route('**/api/me', (route) => route.fulfill({
    status: meStatus,
    contentType: meStatus === 200 ? 'application/json' : 'text/plain',
    body: meStatus === 200 ? JSON.stringify({ id: 'alice', username: 'alice' }) : 'identity service unavailable',
  }));
  await page.routeWebSocket(/\/ws/, () => {});
  await page.goto('/app-harness.html');
  await page.waitForFunction(() => 'authStore' in window);
  await expect.poll(() => page.evaluate(() => Boolean((window as typeof window & { authStore: typeof import('../../src/stores/authStore').useAuthStore }).authStore.getState().user))).toBe(true);
  meStatus = 503;
  await page.evaluate(() => (window as typeof window & { authStore: typeof import('../../src/stores/authStore').useAuthStore }).authStore.getState().checkAuth());
  await expect(page.getByRole('alert')).toContainText('identity service unavailable');
  await expect(page.getByRole('button', { name: 'Retry sign-in check' })).toBeVisible();
  expect(await page.evaluate(() => (window as typeof window & { authStore: typeof import('../../src/stores/authStore').useAuthStore }).authStore.getState().user?.id)).toBe('alice');

  meStatus = 200;
  await page.getByRole('button', { name: 'Retry sign-in check' }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('hidden tabs honor notification settings and claim one desktop alert per message', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    (window as typeof window & { notifications: Array<{ title: string; body?: string }> }).notifications = [];
    class FakeNotification {
      static permission = 'granted';
      constructor(title: string, options?: NotificationOptions) {
        (window as typeof window & { notifications: Array<{ title: string; body?: string }> }).notifications.push({
          title, body: options?.body,
        });
      }
    }
    Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
  });
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore; notifications: unknown[] }).chatStore;
    const channel = {
      id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
      member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
      thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
    };
    store.setState({
      activeAccountId: 'alice', nickname: 'alice', channels: { server: [channel] },
      notificationSettings: { server: [{
        id: 'setting', server_id: 'server', channel_id: null, level: 'mentions',
        suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null,
      }] },
    });
    const projection = {
      type: 'durable_event' as const,
      event: {
        kind: 'message_created', conversation_id: 'conversation', entity_type: 'message', entity_id: 'message',
        entity_version: 1, reaction: null, read_state: null, descriptor: {},
        message: {
          attachments: [], content: 'hello @alice', content_format: 'markdown', conversation_id: 'conversation',
          created_at: '2026-09-06T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
          mentions: [{ kind: 'user' as const, target_id: 'alice', start_byte: 6, end_byte: 12 }],
          message_id: 'message', reply_to: null, reply_to_id: null, sender_id: 'bob', sender_nick: 'bob', sequence: '1',
        },
      },
    };
    store.getState().handleEvent(projection);
    // Simulate a second tab that has not projected the message yet. The shared
    // localStorage claim must still prevent a duplicate desktop notification.
    store.setState({ messages: {}, entityVersions: {} });
    store.getState().handleEvent(projection);
    store.setState({
      memberRoles: { server: { alice: ['role-1'] } },
      messages: {}, entityVersions: {},
    });
    store.getState().handleEvent({ ...projection, event: {
      ...projection.event, entity_id: 'role-message',
      message: { ...projection.event.message, message_id: 'role-message', sequence: '2', content: 'structured role',
        mentions: [{ kind: 'role', target_id: 'role-1', start_byte: 0, end_byte: 0 }] },
    } });
    store.getState().handleEvent({ ...projection, event: {
      ...projection.event, entity_id: 'text-only-message',
      message: { ...projection.event.message, message_id: 'text-only-message', sequence: '3', content: 'hello @alice', mentions: [] },
    } });
    store.setState({
      notificationSettings: { server: [
        { id: 'global', server_id: null, channel_id: null, level: 'all', suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null },
        { id: 'server-default', server_id: 'server', channel_id: null, level: 'default', suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null },
        { id: 'channel-default', server_id: 'server', channel_id: 'channel', level: 'default', suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null },
      ] },
    });
    store.getState().handleEvent({ ...projection, event: {
      ...projection.event, entity_id: 'inherited-message',
      message: { ...projection.event.message, message_id: 'inherited-message', sequence: '4', content: 'inherited all', mentions: [] },
    } });
  });
  await expect.poll(() => page.evaluate(() =>
    (window as typeof window & { notifications: unknown[] }).notifications,
  )).toEqual([
    { title: 'bob', body: 'hello @alice' },
    { title: 'bob', body: 'structured role' },
    { title: 'bob', body: 'inherited all' },
  ]);
});

test('reconnected role membership enables role alerts until a live removal', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    (window as typeof window & { notifications: string[] }).notifications = [];
    class FakeNotification {
      static permission = 'granted';
      constructor(_title: string, options?: NotificationOptions) {
        (window as typeof window & { notifications: string[] }).notifications.push(options?.body ?? '');
      }
    }
    Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
  });
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const handleEvent = store.getState().handleEvent as (event: unknown) => void;
    store.setState({
      activeAccountId: 'alice', nickname: 'alice', memberRoles: {},
      channels: { server: [{
        id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
        member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
        thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
      }] },
      notificationSettings: { server: [{
        id: 'setting', server_id: 'server', channel_id: null, level: 'mentions',
        suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null,
      }] },
    });
    // The authoritative NAMES snapshot is replayed after reconnect and must
    // restore the current account's role assignments before messages arrive.
    handleEvent({
      type: 'names', server_id: 'server', channel: '#general', members: [{
        nickname: 'alice', user_id: 'alice', avatar_url: null, server_avatar_url: null,
        status: 'online', custom_status: null, status_emoji: null, role_ids: ['role-on-call'],
      }],
    });
    const projection = {
      type: 'durable_event',
      event: {
        kind: 'message_created', conversation_id: 'conversation', entity_type: 'message',
        entity_id: 'role-after-reconnect', entity_version: 1, reaction: null, read_state: null, descriptor: {},
        message: {
          attachments: [], content: 'assigned role alert', content_format: 'plain', conversation_id: 'conversation',
          created_at: '2026-09-06T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
          mentions: [{ kind: 'role', target_id: 'role-on-call', start_byte: 0, end_byte: 0 }],
          message_id: 'role-after-reconnect', reply_to: null, reply_to_id: null,
          sender_id: 'bob', sender_nick: 'bob', sequence: '1',
        },
      },
    };
    handleEvent(projection);
    handleEvent({ type: 'member_role_update', server_id: 'server', version: 1, user_id: 'alice', role_ids: [] });
    handleEvent({ ...projection, event: {
      ...projection.event, entity_id: 'role-after-removal',
      message: { ...projection.event.message, message_id: 'role-after-removal', sequence: '2', content: 'removed role alert' },
    } });
  });
  await expect.poll(() => page.evaluate(() =>
    (window as typeof window & { notifications: string[] }).notifications,
  )).toEqual(['assigned role alert']);
});

test('late role snapshots cannot resurrect removed assignments or colors', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const handleEvent = store.getState().handleEvent as (event: unknown) => void;
    const role = {
      id: 'colored', server_id: 'server', name: 'Colored', color: '#123456', icon_url: null,
      position: 1, permissions: 0, is_default: false,
    };
    // Capture the committed version 1 snapshot, then deliver version 2 first.
    const members = Array.from({ length: 300 }, (_, index) => ({
      user_id: `user-${index}`, role_ids: ['colored'],
    }));
    handleEvent({ type: 'role_list', server_id: 'server', version: 1, roles: [role], member_roles: members });
    const heldOld = {
      type: 'role_list', server_id: 'server', version: 1, roles: [role],
      member_roles: members,
    };
    handleEvent({ type: 'role_list', server_id: 'server', version: 2, roles: [], member_roles: undefined });
    handleEvent({ type: 'member_role_update', server_id: 'server', version: 2, user_id: 'alice', role_ids: [] });
    handleEvent(heldOld);
    handleEvent({ type: 'member_role_update', server_id: 'server', version: 1, user_id: 'alice', role_ids: ['colored'] });
    return {
      roles: store.getState().roles.server,
      assignments: store.getState().memberRoles.server?.alice,
      remainingAssignments: Object.values(store.getState().memberRoles.server ?? {}).flat().length,
      memberCount: Object.keys(store.getState().memberRoles.server ?? {}).length,
      version: store.getState().roleProjectionVersions.server,
    };
  });
  expect(result).toEqual({
    roles: [], assignments: [], remainingAssignments: 0, memberCount: 301, version: 2,
  });
});

test('concurrent tabs atomically claim one desktop notification', async ({ context }) => {
  await context.addInitScript(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    (window as typeof window & { notifications: string[] }).notifications = [];
    class FakeNotification {
      static permission = 'granted';
      constructor(title: string) {
        (window as typeof window & { notifications: string[] }).notifications.push(title);
      }
    }
    Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
  });
  const pages = await Promise.all([context.newPage(), context.newPage()]);
  await Promise.all(pages.map(async (page) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
  }));
  await Promise.all(pages.map((page) => page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    store.setState({
      activeAccountId: 'atomic-alice', nickname: 'alice',
      channels: { server: [{
        id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
        member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
        thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
      }] },
      notificationSettings: { server: [{
        id: 'setting', server_id: 'server', channel_id: null, level: 'all',
        suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null,
      }] },
    });
    store.getState().handleEvent({
      type: 'durable_event', event: {
        kind: 'message_created', conversation_id: 'conversation', entity_type: 'message', entity_id: 'atomic-message',
        entity_version: 1, reaction: null, read_state: null, descriptor: {},
        message: {
          attachments: [], content: 'one alert', content_format: 'plain', conversation_id: 'conversation',
          created_at: '2026-09-06T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
          mentions: [], message_id: 'atomic-message', reply_to: null, reply_to_id: null,
          sender_id: 'bob', sender_nick: 'bob', sequence: '1',
        },
      },
    });
  })));
  await expect.poll(async () => (await Promise.all(pages.map((page) => page.evaluate(() =>
    (window as typeof window & { notifications: string[] }).notifications.length,
  )))).reduce((sum, count) => sum + count, 0)).toBe(1);
});

test('channel read state advances only after the conversation tab becomes visible', async ({ page }) => {
  await page.addInitScript(() => {
    (window as typeof window & { testVisibility: DocumentVisibilityState }).testVisibility = 'hidden';
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => (window as typeof window & { testVisibility: DocumentVisibilityState }).testVisibility,
    });
  });
  await page.goto('/layout-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sent: Array<{ type: string }> };
    stores.sent = [];
    stores.chatStore.setState({
      activeAccountId: 'alice', nickname: 'alice', operationGeneration: 'generation',
      ws: { send: (command: { type: string }) => { stores.sent.push(structuredClone(command)); return true; } },
      channels: { server: [{
        id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
        member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
        thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
      }] },
      messages: { 'server:#general': [{
        id: 'message', from: 'bob', sender_id: 'bob', sequence: '7', content: 'unread',
        timestamp: '2026-09-06T00:00:00Z', attachments: [],
      }] },
    });
    stores.uiStore.getState().setActiveServer('server');
    stores.uiStore.getState().setActiveChannel('#general');
  });
  await page.waitForTimeout(50);
  expect(await page.evaluate(() => (window as typeof window & { sent: Array<{ type: string }> }).sent
    .filter((command) => command.type === 'mark_read').length)).toBe(0);
  await page.evaluate(() => {
    (window as typeof window & { testVisibility: DocumentVisibilityState }).testVisibility = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect.poll(() => page.evaluate(() => (window as typeof window & { sent: Array<{ type: string }> }).sent
    .filter((command) => command.type === 'mark_read').length)).toBe(1);
});

test('more than one hundred visible conversations synchronize in bounded complete windows', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sent: Array<{ type: string; request_id?: string; subscriptions?: string[] }> = [];
    store.setState({ ws: { send: (command: { type: string; request_id?: string; subscriptions?: string[] }) => {
      sent.push(structuredClone(command)); return true;
    } } });
    const channels = Array.from({ length: 205 }, (_, index) => ({
      id: `channel-${index}`, conversation_id: `conversation-${index}`, server_id: 'server',
      name: `channel-${index}`, topic: '', member_count: 1, category_id: null, position: index,
      is_private: false, channel_type: 'text', thread_parent_message_id: null, archived: false,
      slowmode_seconds: 0, is_nsfw: false,
    }));
    store.getState().handleEvent({ type: 'channel_list', server_id: 'server', channels });
    const syncs = sent.filter((command) => command.type === 'sync');
    const respond = (sync: typeof syncs[number], omitFirst = false) => {
      const subscriptions = (sync.subscriptions ?? []).slice(omitFirst ? 1 : 0);
      store.getState().handleEvent({ type: 'sync_snapshot', request_id: sync.request_id!, snapshot: {
        cursor: `cursor-${sync.request_id}`, operation_generation: 'generation', protocol_version: 2,
        history_before: {}, reactions: [], read_states: [], messages: subscriptions.map((conversation, index) => ({
          attachments: [], content: conversation, content_format: 'plain', conversation_id: conversation,
          created_at: '2026-09-01T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
          mentions: [], message_id: `message-${conversation}`, reply_to: null, reply_to_id: null,
          sender_id: 'peer', sender_nick: 'peer', sequence: String(index + 1),
        })),
      } });
    };
    syncs.forEach((sync) => respond(sync));
    const firstCount = Object.values(store.getState().messages).flat().length;
    store.getState().handleEvent({ type: 'channel_list', server_id: 'server', channels: channels.slice(1) });
    const repeated = sent.filter((command) => command.type === 'sync').slice(-3);
    repeated.forEach((sync) => respond(sync));
    return {
      sizes: syncs.map((command) => command.subscriptions?.length),
      subscriptions: new Set(syncs.flatMap((command) => command.subscriptions ?? [])).size,
      firstCount,
      repeatedCount: Object.values(store.getState().messages).flat().length,
    };
  });
  expect(result).toEqual({
    sizes: [100, 100, 5], subscriptions: 205, firstCount: 205, repeatedCount: 204,
  });
});

test('logout isolates drafts by durable account and restores them on return', async ({ page }) => {
  await page.routeWebSocket(/\/ws\?nickname=/, (socket) => socket.onMessage(() => {}));
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

test('a nonretryable server rejection removes the optimistic row without overwriting newer composition', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window);
  const result = await page.evaluate(() => {
    const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
    const sent: Array<{ request_id: string }> = [];
    store.setState({
      nickname: 'carmilla', activeAccountId: 'account-a', operationGeneration: 'generation-2',
      ws: { send: (command: { request_id: string }) => { sent.push(command); return true; } },
    });
    store.getState().sendMessage('server-1', 'general', 'rejected composition');
    store.getState().setDraft('server-1:general', 'newer composition');
    store.getState().handleEvent({
      type: 'command_error', request_id: sent[0].request_id, code: 'INVALID_INPUT',
      message: 'rejected', retryable: false,
    });
    return {
      draft: store.getState().drafts['server-1:general'],
      messages: store.getState().messages['server-1:general'] ?? [],
      pending: Object.keys(store.getState().pendingCommands),
    };
  });
  expect(result).toEqual({ draft: 'newer composition', messages: [], pending: [] });
});

test('mounted composer never sends an upload through a different account socket', async ({ page }) => {
  let releaseUpload!: () => void;
  const uploadReleased = new Promise<void>((resolve) => { releaseUpload = resolve; });
  let markStarted!: () => void;
  const uploadStarted = new Promise<void>((resolve) => { markStarted = resolve; });
  await page.route(/\/api\/uploads/, async (route) => {
    markStarted();
    await uploadReleased;
    await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify({
      id: 'attachment-1', filename: 'proof.txt', content_type: 'text/plain', file_size: 5,
      url: '/api/uploads/attachment-1',
    }) });
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

test('a held voice failure stays with its captured account and conversation', async ({ page }) => {
  let releaseUpload!: () => void;
  const released = new Promise<void>((resolve) => { releaseUpload = resolve; });
  let markStarted!: () => void;
  const started = new Promise<void>((resolve) => { markStarted = resolve; });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: {
      getUserMedia: async () => ({ getTracks: () => [{ stop: () => {} }] }),
    } });
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
    scope.chatStore.setState({ nickname: 'alice', activeAccountId: 'account-a', accountGeneration: 1,
      operationGeneration: 'generation-a', ws: { send: () => true } });
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

test('reply targets remain scoped while navigating between conversations', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
    scope.chatStore.setState({ messages: {
      'server:alpha': [{ id: 'alpha-message', from: 'alice', content: 'alpha reply', timestamp: '2026-09-06T00:00:00Z' }],
      'server:beta': [{ id: 'beta-message', from: 'bob', content: 'beta reply', timestamp: '2026-09-06T00:01:00Z' }],
    } });
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

test('failed composer upload stays attached and can be retried visibly', async ({ page }) => {
  let attempts = 0;
  await page.route(/\/api\/uploads/, async (route) => {
    attempts += 1;
    if (attempts === 1) {
      await route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Media storage is temporarily unavailable' }) });
      return;
    }
    await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify({
      id: 'attachment-retried', filename: 'retry.txt', content_type: 'text/plain', file_size: 5,
      url: '/api/uploads/attachment-retried',
    }) });
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

test('search validates input and uses correlated continuation pages with deletion tombstones', async ({ page }) => {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
  await page.evaluate(() => {
    const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sentCommands: unknown[] };
    stores.sentCommands = [];
    stores.chatStore.setState({ ws: { send: (command: unknown) => { stores.sentCommands.push(structuredClone(command)); return true; } } });
    stores.uiStore.getState().setActiveServer('server-1');
    stores.uiStore.getState().setShowSearch(true);
  });
  const search = page.getByPlaceholder('Search messages...');
  await search.fill('before:yesterday');
  await search.press('Enter');
  await expect(page.getByRole('alert')).toContainText('Invalid before: date');
  expect(await page.evaluate(() => (window as typeof window & { sentCommands: unknown[] }).sentCommands)).toEqual([]);

  await search.fill('from:alice release notes');
  await search.press('Enter');
  await expect.poll(() => page.evaluate(() => {
    const commands = (window as typeof window & { sentCommands: Array<{ type?: string; limit?: number; offset?: number }> }).sentCommands;
    return commands.at(-1);
  })).toMatchObject({ type: 'search_messages', limit: 25, offset: 0 });
  await page.evaluate(() => {
    const scope = window as typeof window & {
      chatStore: ChatStore;
      sentCommands: Array<{ request_id?: string }>;
    };
    const requestId = scope.sentCommands.at(-1)!.request_id!;
    scope.chatStore.getState().handleEvent({
      type: 'search_results', request_id: requestId, server_id: 'server-1',
      query: 'from:alice release notes', results: [{
        id: 'deleted-result', from: 'alice', content: 'old', timestamp: '2026-01-01T00:00:00Z',
        channel_id: 'channel-1', channel_name: 'general',
      }], total_count: 60, offset: 0, next_continuation: 'page-2', restarted: false,
    });
  });
  await page.getByRole('button', { name: 'Next' }).click();
  await expect.poll(() => page.evaluate(() => {
    const commands = (window as typeof window & { sentCommands: Array<{ type?: string; offset?: number; continuation?: string }> }).sentCommands;
    return commands.at(-1);
  })).toMatchObject({ offset: 25, continuation: 'page-2' });
  const state = await page.evaluate(() => {
    const scope = window as typeof window & {
      chatStore: ChatStore;
      sentCommands: Array<{ request_id?: string }>;
    };
    const staleRequest = scope.sentCommands.at(-2)!.request_id!;
    const currentRequest = scope.sentCommands.at(-1)!.request_id!;
    scope.chatStore.getState().handleEvent({
      type: 'message_delete', server_id: 'server-1', channel: '#general', id: 'deleted-result',
    });
    const stalePage = {
      type: 'search_results' as const, server_id: 'server-1', query: 'from:alice release notes',
      results: [{ id: 'deleted-result', from: 'alice', content: 'stale', timestamp: '2026-01-01T00:00:00Z', channel_id: 'channel-1', channel_name: 'general' }],
      total_count: 60, offset: 0, next_continuation: 'stale-token', restarted: false,
    };
    scope.chatStore.getState().handleEvent({ ...stalePage, request_id: staleRequest });
    scope.chatStore.getState().handleEvent({ ...stalePage, request_id: currentRequest, offset: 25, next_continuation: null });
    return {
      results: scope.chatStore.getState().searchResults,
      deleted: scope.chatStore.getState().deletedMessageIds,
      offset: scope.chatStore.getState().searchOffset,
    };
  });
  expect(state).toEqual({ results: [], deleted: { 'deleted-result': true }, offset: 25 });
});
