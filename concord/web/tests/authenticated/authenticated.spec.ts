import { expect, test } from '@playwright/test';
import { existsSync, readFileSync, unlinkSync, writeFileSync } from 'node:fs';
import { createConnection } from 'node:net';
import WebSocket from 'ws';

const sessions = JSON.parse(readFileSync(process.env.CONCORD_AUTH_SESSIONS_FILE!, 'utf8')) as Record<'alice' | 'alice_revoke' | 'bob' | 'bob_irc' | 'helper_bot' | 'helper_bot_token_id' | 'limited_helper_bot' | 'wrong_bot' | 'historical_non_uuid_message_id' | 'historical_padded_message_id' | 'historical_long_message_id', string>;

async function ircClient() {
  const socket = createConnection({ host: '127.0.0.1', port: Number(process.env.CONCORD_IRC_PORT) });
  const lines: string[] = [];
  const waiters: Array<{ predicate: (line: string) => boolean; resolve: (line: string) => void }> = [];
  let buffered = '';
  socket.setEncoding('utf8');
  socket.on('data', (chunk) => {
    buffered += chunk;
    const complete = buffered.split(/\r?\n/);
    buffered = complete.pop() ?? '';
    for (const line of complete) {
      lines.push(line);
      const index = waiters.findIndex((waiter) => waiter.predicate(line));
      if (index >= 0) waiters.splice(index, 1)[0].resolve(line);
    }
  });
  await new Promise<void>((resolve, reject) => {
    socket.once('connect', resolve);
    socket.once('error', reject);
  });
  const waitFor = (predicate: (line: string) => boolean) => {
    const existing = lines.find(predicate);
    if (existing) return Promise.resolve(existing);
    return Promise.race([
      new Promise<string>((resolve) => waiters.push({ predicate, resolve })),
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`IRC line timed out; received: ${lines.join(' | ')}`)), 5_000)),
    ]);
  };
  const send = (line: string) => socket.write(`${line}\r\n`);
  return { socket, send, waitFor };
}

async function registerIrc(client: Awaited<ReturnType<typeof ircClient>>, tagged = false) {
  if (tagged) {
    client.send('CAP LS 302');
    await client.waitFor((line) => line.includes(' CAP * LS :') && line.includes('server-time') && line.includes('message-tags'));
    client.send('CAP REQ :server-time message-tags');
    await client.waitFor((line) => line.includes(' CAP * ACK :server-time message-tags'));
    client.send('CAP END');
  }
  client.send(`PASS ${sessions.bob_irc}`);
  client.send('NICK bob');
  client.send('USER bob 0 * :Bob');
  const welcome = await client.waitFor((line) => line.includes(' 001 '));
  const nick = welcome.split(' ')[2];
  if (!nick) throw new Error(`IRC welcome returned no nickname: ${welcome}`);
  client.send('LIST');
  const listed = await client.waitFor((line) => line.includes(` 322 ${nick} `));
  const channel = listed.split(' ')[3];
  if (!channel?.startsWith('#')) throw new Error(`IRC LIST returned no channel: ${listed}`);
  return channel;
}

function decodeIrcTagValue(value: string) {
  let decoded = '';
  for (let index = 0; index < value.length; index += 1) {
    if (value[index] !== '\\' || index + 1 >= value.length) {
      decoded += value[index];
      continue;
    }
    index += 1;
    decoded += ({ ':': ';', s: ' ', '\\': '\\', r: '\r', n: '\n' } as Record<string, string>)[value[index]] ?? value[index];
  }
  return decoded;
}

function ircTags(line: string) {
  if (!line.startsWith('@')) return {};
  const separator = line.indexOf(' ');
  return Object.fromEntries(line.slice(1, separator).split(';').map((tag) => {
    const equals = tag.indexOf('=');
    return equals < 0
      ? [tag, '']
      : [tag.slice(0, equals), decodeIrcTagValue(tag.slice(equals + 1))];
  }));
}

function captureSocketDiagnostics(page: import('@playwright/test').Page, label: string, pageErrors: string[]) {
  page.on('console', (message) => console.log(`[${label}:console:${message.type()}] ${message.text()}`));
  page.on('pageerror', (error) => {
    pageErrors.push(error.message);
    console.log(`[${label}:pageerror] ${error.message}`);
  });
  page.on('websocket', (socket) => {
    socket.on('framereceived', (event) => console.log(`[${label}:ws:received] ${String(event.payload).slice(0, 500)}`));
    socket.on('framesent', (event) => console.log(`[${label}:ws:sent] ${String(event.payload).slice(0, 500)}`));
    socket.on('socketerror', (error) => console.log(`[${label}:ws:error] ${error}`));
    socket.on('close', () => console.log(`[${label}:ws:close]`));
  });
}

async function openGeneral(page: import('@playwright/test').Page) {
  await page.getByTitle('Browser fixture').click();
  await page.getByRole('button', { name: 'general' }).click();
}

async function attachRawSocket(page: import('@playwright/test').Page) {
  await page.evaluate(() => {
    const state = window as typeof window & { rawSocket?: WebSocket; rawFrames?: unknown[] };
    state.rawFrames = [];
    state.rawSocket = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/ws`);
    state.rawSocket.addEventListener('message', (event) => {
      try { state.rawFrames!.push(JSON.parse(String(event.data))); } catch { /* ignore non-JSON diagnostics */ }
    });
  });
  await page.waitForFunction(() => (window as typeof window & { rawSocket?: WebSocket }).rawSocket?.readyState === WebSocket.OPEN);
}

async function rawSend(page: import('@playwright/test').Page, message: unknown) {
  await page.evaluate((payload) => {
    (window as typeof window & { rawSocket?: WebSocket }).rawSocket!.send(JSON.stringify(payload));
  }, message);
}

async function rawFramesFromPage(page: import('@playwright/test').Page) {
  return page.evaluate(() => (window as typeof window & { rawFrames?: unknown[] }).rawFrames ?? []);
}

test('invalid search continuation is correlated over the real WebSocket', async ({ browser, baseURL }) => {
  const context = await browser.newContext();
  await context.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await context.newPage();
  await page.goto('/');
  await attachRawSocket(page);
  await rawSend(page, {
    type: 'search_messages', request_id: 'invalid-search-cursor', server_id: 'browser-server',
    query: 'needle', limit: 25, offset: 0, continuation: 'invalid',
  });
  await expect.poll(async () => (await rawFramesFromPage(page)).find((frame) =>
    typeof frame === 'object' && frame !== null
      && (frame as { type?: string }).type === 'command_error')).toMatchObject({
    type: 'command_error', request_id: 'invalid-search-cursor',
    code: 'INVALID_CONTINUATION', retryable: false,
  });
  await context.close();
});

async function botSocket(baseURL: string, token: string) {
  const frames: unknown[] = [];
  const socket = new WebSocket(baseURL.replace(/^http/, 'ws') + '/ws', {
    headers: { Authorization: `Bearer ${token}`, Origin: baseURL },
  });
  socket.on('message', (data) => {
    try { frames.push(JSON.parse(data.toString())); } catch { /* ignore non-JSON diagnostics */ }
  });
  let resolveClosed!: () => void;
  const closed = new Promise<void>((resolve) => { resolveClosed = resolve; });
  socket.once('close', resolveClosed);
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
    socket.once('unexpected-response', (_request, response) => reject(new Error(`WebSocket rejected with ${response.statusCode}`)));
  });
  return { socket, frames, closed, send: (message: unknown) => socket.send(JSON.stringify(message)) };
}

const isErrorFrame = (frame: unknown) => ['error', 'command_error'].includes((frame as { type?: string }).type ?? '');

test('installed bot completes public and private component journeys on real browser sockets', async ({ browser, baseURL }) => {
  await expect(botSocket(baseURL!, 'invalid-bot-token')).rejects.toThrow(/401/);
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const bobPage = await bob.newPage();
  const [helperBot, limitedBot, wrongBot] = await Promise.all([
    botSocket(baseURL!, sessions.helper_bot),
    botSocket(baseURL!, sessions.limited_helper_bot),
    botSocket(baseURL!, sessions.wrong_bot),
  ]);
  for (const deniedBot of [limitedBot, wrongBot]) {
    deniedBot.send({ type: 'fetch_history', server_id: 'browser-server', channel: '#general', limit: 50 });
    await expect.poll(() => deniedBot.frames.filter(isErrorFrame).length).toBe(1);
    expect(deniedBot.frames.filter((frame) => (frame as { type?: string }).type === 'history')).toEqual([]);
  }
  await Promise.all([alicePage.goto('/'), bobPage.goto('/')]);
  await Promise.all([openGeneral(alicePage), openGeneral(bobPage)]);
  await Promise.all([attachRawSocket(alicePage), attachRawSocket(bobPage)]);

  const composer = alicePage.getByPlaceholder('Message #general');
  await composer.fill('/journey');
  await expect(alicePage.getByRole('option', { name: /journey/ })).toBeVisible();
  await composer.press('Enter');
  await expect.poll(() => helperBot.frames.filter((frame) => (frame as { type?: string }).type === 'interaction_create').length).toBe(1);
  const slash = helperBot.frames.find((frame) => (frame as { type?: string }).type === 'interaction_create') as { interaction: { id: string } };

  const wrongErrorsBefore = wrongBot.frames.filter(isErrorFrame).length;
  wrongBot.send({ type: 'respond_to_interaction', interaction_id: slash.interaction.id, content: 'wrong bot', ephemeral: false });
  await expect.poll(() => wrongBot.frames.filter(isErrorFrame).length).toBe(wrongErrorsBefore + 1);

  const helperErrorsBefore = helperBot.frames.filter(isErrorFrame).length;
  helperBot.send({ type: 'respond_to_interaction', interaction_id: 'browser-expired', content: 'too late', ephemeral: false });
  await expect.poll(() => helperBot.frames.filter(isErrorFrame).length).toBe(helperErrorsBefore + 1);

  helperBot.send({
    type: 'respond_to_interaction', interaction_id: slash.interaction.id, content: 'Public controls', ephemeral: false,
    components_json: JSON.stringify([{ type: 'action_row', components: [{ type: 'button', custom_id: 'continue', label: 'Continue', style: 'primary' }] }]),
  });
  await Promise.all([
    expect(alicePage.getByText('Public controls')).toBeVisible(),
    expect(bobPage.getByText('Public controls')).toBeVisible(),
  ]);
  await alicePage.getByRole('button', { name: 'Continue' }).click();
  await expect.poll(() => helperBot.frames.filter((frame) => (frame as { interaction?: { interaction_type?: string } }).interaction?.interaction_type === 'button').length).toBe(1);
  const button = helperBot.frames.find((frame) => (frame as { interaction?: { interaction_type?: string } }).interaction?.interaction_type === 'button') as { interaction: { id: string } };

  helperBot.send({
    type: 'respond_to_interaction', interaction_id: button.interaction.id, content: 'Private choice', ephemeral: true,
    components_json: JSON.stringify([{ type: 'action_row', components: [{ type: 'select_menu', custom_id: 'choice', placeholder: 'Choose privately', min_values: 1, max_values: 1, options: [{ label: 'One', value: 'one' }, { label: 'Two', value: 'two' }] }] }]),
  });
  await expect(alicePage.getByText('Private choice')).toBeVisible();
  const bobServerListsBefore = (await rawFramesFromPage(bobPage)).filter((frame) => (frame as { type?: string }).type === 'server_list').length;
  await rawSend(bobPage, { type: 'list_servers' });
  await expect.poll(async () => (await rawFramesFromPage(bobPage)).filter((frame) => (frame as { type?: string }).type === 'server_list').length).toBe(bobServerListsBefore + 1);
  await expect(bobPage.getByText('Private choice')).toHaveCount(0);
  expect((await rawFramesFromPage(bobPage)).filter((frame) => (frame as { type?: string }).type === 'interaction_response')).toEqual([]);
  await alicePage.getByRole('combobox', { name: 'Choose privately' }).selectOption('two');
  await expect.poll(() => helperBot.frames.filter((frame) => (frame as { interaction?: { interaction_type?: string } }).interaction?.interaction_type === 'select_menu').length).toBe(1);
  const selectFrame = helperBot.frames.find((frame) => (frame as { interaction?: { interaction_type?: string } }).interaction?.interaction_type === 'select_menu') as { interaction: { data: { values: string[] } } };
  expect(selectFrame.interaction.data.values).toEqual(['two']);

  const errorsBeforeReplay = helperBot.frames.filter(isErrorFrame).length;
  helperBot.send({ type: 'respond_to_interaction', interaction_id: button.interaction.id, content: 'replay', ephemeral: true });
  await expect.poll(() => helperBot.frames.filter(isErrorFrame).length).toBe(errorsBeforeReplay + 1);
  await rawSend(alicePage, { type: 'delete_bot_token', token_id: sessions.helper_bot_token_id });
  await Promise.race([
    helperBot.closed,
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error('revoked bot socket did not close')), 5_000)),
  ]);
  await expect(botSocket(baseURL!, sessions.helper_bot)).rejects.toThrow(/401/);
  expect(wrongBot.socket.readyState).toBe(WebSocket.OPEN);
  expect(await bobPage.evaluate(() => fetch('/api/me').then((response) => response.status))).toBe(200);
  limitedBot.socket.close();
  wrongBot.socket.close();
  await Promise.all([alice.close(), bob.close()]);
});

test('real authenticated browser connectivity smoke uses isolated accounts', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const bobPage = await bob.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(alicePage, 'alice', pageErrors);
  captureSocketDiagnostics(bobPage, 'bob', pageErrors);
  await Promise.all([alicePage.goto('/'), bobPage.goto('/')]);
  const [aliceApi, bobApi] = await Promise.all([alicePage, bobPage].map((page) => page.evaluate(async () => {
    const [me, servers] = await Promise.all([
      fetch('/api/me').then((response) => response.json()),
      fetch('/api/servers').then((response) => response.json()),
    ]);
    return { me, servers };
  })));
  expect(aliceApi).toMatchObject({ me: { id: 'browser-alice', username: 'alice' }, servers: [{ name: 'Browser fixture' }] });
  expect(bobApi).toMatchObject({ me: { id: 'browser-bob', username: 'bob' }, servers: [{ name: 'Browser fixture' }] });
  await expect(alicePage.locator('body')).not.toContainText('Sign in with AT Protocol');
  await expect(bobPage.locator('body')).not.toContainText('Sign in with AT Protocol');
  expect(pageErrors).toEqual([]);
  await Promise.all([alice.close(), bob.close()]);
});

test('server creation settings and integrations dialogs trap Escape and restore focus', async ({ browser, baseURL }) => {
  const context = await browser.newContext();
  await context.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await context.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'dialogs', pageErrors);
  await page.goto('/');

  const createTrigger = page.getByRole('button', { name: 'Create a server' });
  await createTrigger.click();
  const createDialog = page.getByRole('dialog', { name: 'Create a Server' });
  await expect(createDialog).toBeVisible();
  await expect(createDialog.getByPlaceholder('My Awesome Server')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(createDialog).toHaveCount(0);
  await expect(createTrigger).toBeFocused();

  await openGeneral(page);
  const settingsTrigger = page.getByRole('button', { name: 'Server Settings' });
  await settingsTrigger.click();
  const settingsDialog = page.getByRole('dialog', { name: 'Browser fixture Settings' });
  await expect(settingsDialog).toBeVisible();
  await page.keyboard.press('Shift+Tab');
  await expect(settingsDialog.locator(':focus')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(settingsDialog).toHaveCount(0);
  await expect(settingsTrigger).toBeFocused();

  const integrationsTrigger = page.getByRole('button', { name: 'Integrations' });
  await integrationsTrigger.click();
  const integrationsDialog = page.getByRole('dialog', { name: 'Integrations' });
  await expect(integrationsDialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(integrationsDialog).toHaveCount(0);
  await expect(integrationsTrigger).toBeFocused();
  expect(pageErrors).toEqual([]);
  await context.close();
});

test('integration lifecycle preserves rejected input and retries an install after reconnect', async ({ browser, baseURL }) => {
  const restartRequest = process.env.CONCORD_RESTART_REQUEST!;
  const restartAck = process.env.CONCORD_RESTART_ACK!;
  if (existsSync(restartAck)) unlinkSync(restartAck);
  const context = await browser.newContext();
  await context.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  await context.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, 'clipboard', {
      configurable: true,
      get: () => ({ writeText: () => Promise.reject(new DOMException('clipboard denied', 'NotAllowedError')) }),
    });
  });
  const page = await context.newPage();
  let appSocketCloses = 0;
  let appSocketOpens = 0;
  let readyAppSockets = 0;
  page.on('websocket', (socket) => {
    if (new URL(socket.url()).pathname !== '/ws') return;
    appSocketOpens += 1;
    let receivedFrame = false;
    socket.on('framereceived', () => {
      if (receivedFrame) return;
      receivedFrame = true;
      readyAppSockets += 1;
    });
    socket.on('close', () => { appSocketCloses += 1; });
  });
  await page.goto('/');
  await openGeneral(page);
  await expect.poll(() => readyAppSockets).toBe(1);
  await page.getByRole('button', { name: 'Integrations' }).click();
  const dialog = page.getByRole('dialog', { name: 'Integrations' });
  await dialog.getByRole('button', { name: 'Bots' }).click();
  await dialog.getByRole('button', { name: 'Create Bot' }).click();
  const username = dialog.getByPlaceholder('Bot username');
  await username.fill('helper-bot');
  await dialog.getByRole('button', { name: 'Create Bot', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Failed to create bot');
  await expect(username).toHaveValue('helper-bot');

  const uniqueName = `lifecycle-bot-${Date.now()}`;
  await username.fill(uniqueName);
  await dialog.getByRole('button', { name: 'Create Bot', exact: true }).click();
  await expect(dialog.getByText('Bot account created.')).toBeVisible();
  await expect(dialog.getByLabel('Bot account')).toHaveValue(/.+/);
  await expect(dialog.getByLabel('Bot account').locator('option:checked')).toHaveText(uniqueName);
  await dialog.getByRole('button', { name: 'Copy token' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Clipboard access failed');

  writeFileSync(restartRequest, `${Date.now()}\n`, { flag: 'wx' });
  await expect.poll(() => appSocketCloses, { timeout: 5_000 }).toBe(1);
  await dialog.getByRole('button', { name: 'Install on server' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Not connected');
  await expect(dialog.getByLabel('Bot account').locator('option:checked')).toHaveText(uniqueName);
  await expect.poll(() => existsSync(restartAck), { timeout: 15_000 }).toBe(true);
  await expect.poll(() => readyAppSockets, { timeout: 10_000 }).toBe(2);
  expect(appSocketOpens).toBeGreaterThanOrEqual(2);
  await dialog.getByRole('button', { name: 'Install on server' }).click();
  await expect(dialog.getByText('Bot installed on server.')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Remove from server' })).toBeVisible();
  await context.close();
});

test('a direct message committed while its recipient is offline is delivered after login', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const pageErrors: string[] = [];
  let directRequestId: string | null = null;
  let resolveCommitted!: (value: { id: string; conversation_id: string; request_id: string }) => void;
  const committed = new Promise<{ id: string; conversation_id: string; request_id: string }>((resolve) => { resolveCommitted = resolve; });
  alicePage.on('websocket', (socket) => {
    socket.on('framesent', ({ payload }) => {
      try {
        const frame = JSON.parse(String(payload)) as { type?: string; request_id?: string };
        if (frame.type === 'send_direct_message' && frame.request_id) directRequestId = frame.request_id;
      } catch { /* diagnostic frames may be binary */ }
    });
    socket.on('framereceived', ({ payload }) => {
      try {
        const frame = JSON.parse(String(payload)) as { type?: string; id?: string; conversation_id?: string; request_id?: string };
        if (frame.type === 'message_ack' && frame.request_id === directRequestId && frame.id && frame.conversation_id) {
          resolveCommitted({ id: frame.id, conversation_id: frame.conversation_id, request_id: frame.request_id });
        }
      } catch { /* diagnostic frames may be binary */ }
    });
  });
  captureSocketDiagnostics(alicePage, 'alice-dm', pageErrors);
  await alicePage.goto('/');

  await alicePage.getByRole('button', { name: 'Direct messages' }).click();
  await alicePage.getByRole('button', { name: /bob/ }).click();
  await expect(alicePage.getByText('offline hello')).toBeVisible();
  const composer = alicePage.getByPlaceholder('Message @bob');
  await composer.fill('real websocket reply');
  await composer.press('Enter');
  await expect(alicePage.getByText('real websocket reply')).toBeVisible();
  const receipt = await Promise.race([
    committed,
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error('Direct-message commit receipt timed out')), 5_000)),
  ]);
  expect(receipt).toMatchObject({ conversation_id: 'browser-dm', request_id: directRequestId });

  const bobPage = await bob.newPage();
  captureSocketDiagnostics(bobPage, 'bob-dm', pageErrors);
  await bobPage.goto('/');
  await bobPage.getByRole('button', { name: 'Direct messages' }).click();
  await bobPage.getByRole('button', { name: /alice/ }).click();
  await expect(bobPage.getByText('real websocket reply')).toBeVisible();
  expect(pageErrors).toEqual([]);
  await Promise.all([alice.close(), bob.close()]);
});

test('restart reconnects multiple tabs to durable history and credential revocation closes only its sessions', async ({ browser, baseURL }) => {
  const restartRequest = process.env.CONCORD_RESTART_REQUEST!;
  const restartAck = process.env.CONCORD_RESTART_ACK!;
  if (existsSync(restartAck)) unlinkSync(restartAck);
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice_revoke, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const aliceOne = await alice.newPage();
  const aliceTwo = await alice.newPage();
  const bobPage = await bob.newPage();
  const pages = [aliceOne, aliceTwo, bobPage];
  const appSocketCloses = new Map(pages.map((page) => [page, 0]));
  const appSocketBootstraps = new Map(pages.map((page) => [page, 0]));
  for (const page of pages) {
    page.on('websocket', (socket) => {
      if (new URL(socket.url()).pathname === '/ws') {
        let bootstrapped = false;
        socket.on('close', () => appSocketCloses.set(page, (appSocketCloses.get(page) ?? 0) + 1));
        socket.on('framereceived', ({ payload }) => {
          try {
            const frame = JSON.parse(String(payload)) as {
              type?: string;
              request_id?: unknown;
              snapshot?: { protocol_version?: unknown; operation_generation?: unknown; cursor?: unknown };
            };
            if (!bootstrapped
              && frame.type === 'sync_snapshot'
              && typeof frame.request_id === 'string'
              && frame.snapshot?.protocol_version === 2
              && typeof frame.snapshot.operation_generation === 'string'
              && typeof frame.snapshot.cursor === 'string') {
              bootstrapped = true;
              appSocketBootstraps.set(page, (appSocketBootstraps.get(page) ?? 0) + 1);
            }
          } catch { /* binary frame */ }
        });
      }
    });
  }
  let requestId: string | null = null;
  let resolveCommit!: (id: string) => void;
  const commit = new Promise<string>((resolve) => { resolveCommit = resolve; });
  aliceOne.on('websocket', (socket) => {
    if (new URL(socket.url()).pathname !== '/ws') return;
    socket.on('framesent', ({ payload }) => {
      try {
        const frame = JSON.parse(String(payload)) as { type?: string; request_id?: string };
        if (frame.type === 'send_message' && frame.request_id) requestId = frame.request_id;
      } catch { /* binary frame */ }
    });
    socket.on('framereceived', ({ payload }) => {
      try {
        const frame = JSON.parse(String(payload)) as { type?: string; request_id?: string; id?: string };
        if (requestId && frame.type === 'message_ack' && frame.request_id === requestId && frame.id) resolveCommit(frame.id);
      } catch { /* binary frame */ }
    });
  });
  await Promise.all(pages.map((page) => page.goto('/')));
  await Promise.all(pages.map(openGeneral));
  const composer = aliceOne.getByPlaceholder('Message #general');
  await composer.fill('durable across an actual server restart');
  await composer.press('Enter');
  const committedId = await Promise.race([
    commit,
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error('Channel commit receipt timed out')), 5_000)),
  ]);

  await Promise.all(pages.map((page) => expect.poll(
    () => appSocketBootstraps.get(page) ?? 0,
  ).toBeGreaterThan(0)));
  const bootstrapsBeforeRestart = new Map(pages.map((page) => [page, appSocketBootstraps.get(page) ?? 0]));
  const closesBeforeRestart = new Map(pages.map((page) => [page, appSocketCloses.get(page) ?? 0]));
  writeFileSync(restartRequest, `${Date.now()}\n`, { flag: 'wx' });
  await expect.poll(() => existsSync(restartAck), { timeout: 15_000 }).toBe(true);
  await Promise.all(pages.map((page) => expect.poll(
    () => appSocketCloses.get(page) ?? 0,
    { timeout: 10_000 },
  ).toBeGreaterThan(closesBeforeRestart.get(page)!)));
  await Promise.all(pages.map((page) => expect.poll(
    () => appSocketBootstraps.get(page) ?? 0,
    { timeout: 10_000 },
  ).toBeGreaterThan(bootstrapsBeforeRestart.get(page)!)));

  const recoveredMessage = `live after restart ${Date.now()}`;
  await composer.fill(recoveredMessage);
  await composer.press('Enter');
  await expect(bobPage.getByText(recoveredMessage, { exact: true })).toBeVisible();

  await aliceOne.reload();
  await openGeneral(aliceOne);
  await expect(aliceOne.locator(`[data-message-id="${committedId}"]`)).toContainText('durable across an actual server restart');

  const closesBeforeRevoke = new Map(pages.map((page) => [page, appSocketCloses.get(page) ?? 0]));
  await aliceOne.evaluate(() => fetch('/api/auth/logout', { method: 'POST' }));
  await expect.poll(() => appSocketCloses.get(aliceOne) ?? 0, { timeout: 5_000 }).toBeGreaterThan(closesBeforeRevoke.get(aliceOne)!);
  await expect.poll(() => appSocketCloses.get(aliceTwo) ?? 0, { timeout: 5_000 }).toBeGreaterThan(closesBeforeRevoke.get(aliceTwo)!);
  expect(await aliceTwo.evaluate(() => fetch('/api/me').then((response) => response.status))).toBe(401);
  expect(await bobPage.evaluate(() => fetch('/api/me').then((response) => response.status))).toBe(200);
  expect(appSocketCloses.get(bobPage)).toBe(closesBeforeRevoke.get(bobPage));
  await Promise.all([alice.close(), bob.close()]);
});

test('IRC and browser share canonical channel delivery, opaque history, and reconnect recovery', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  await alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await alice.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'alice-irc', pageErrors);
  const first = await ircClient();
  let second: Awaited<ReturnType<typeof ircClient>> | null = null;
  try {
    await page.goto('/');
    await openGeneral(page);
    const channel = await registerIrc(first);
    first.send(`JOIN ${channel}`);
    await first.waitFor((line) => line.includes(` JOIN ${channel}`));
    first.send(`PRIVMSG ${channel} :message from a real IRC stream`);
    await expect(page.getByText('message from a real IRC stream')).toBeVisible();

    const composer = page.getByPlaceholder('Message #general');
    await composer.fill('browser response to IRC');
    await composer.press('Enter');
    await first.waitFor((line) => line.includes(`PRIVMSG ${channel} :browser response to IRC`));
    first.send('QUIT :reconnect test');
    await first.waitFor((line) => line.includes('ERROR :Closing Link'));
    first.socket.destroy();

    second = await ircClient();
    const reconnectedChannel = await registerIrc(second, true);
    second.send(`HISTORY ${reconnectedChannel} 20`);
    await second.waitFor((line) => line.includes(`PRIVMSG ${reconnectedChannel} :browser response to IRC`));

    const historical = [
      {
        id: sessions.historical_non_uuid_message_id,
        content: 'historical legacy timestamp',
        ircTime: '2024-01-02T03:04:05.123456+00:00',
        browserTime: '2024-01-02T03:04:05.123456Z',
      },
      {
        id: sessions.historical_padded_message_id,
        content: 'historical offset timestamp',
        ircTime: '2024-01-02T08:04:06.654321+00:00',
        browserTime: '2024-01-02T08:04:06.654321Z',
      },
      {
        id: sessions.historical_long_message_id,
        content: 'historical long unicode identifier',
        ircTime: '2024-01-02T08:04:07.987654321+00:00',
        browserTime: '2024-01-02T08:04:07.987654321Z',
      },
    ];
    expect(new TextEncoder().encode(sessions.historical_long_message_id).length).toBeGreaterThan(512);
    for (const expected of historical) {
      const line = await second.waitFor((candidate) => candidate.includes(`PRIVMSG ${reconnectedChannel} :${expected.content}`));
      expect(ircTags(line)).toMatchObject({ time: expected.ircTime, msgid: expected.id });
      await expect(page.getByText(expected.content)).toBeVisible();
    }
    const browserIds = await page.locator('[data-message-id]').evaluateAll((elements) =>
      elements.map((element) => element.getAttribute('data-message-id')),
    );
    for (const expected of historical) expect(browserIds).toContain(expected.id);

    await attachRawSocket(page);
    await rawSend(page, { type: 'fetch_history', server_id: 'browser-server', channel: '#general', limit: 20 });
    await expect.poll(async () => (await rawFramesFromPage(page)).find((frame) =>
      (frame as { type?: string }).type === 'history')).toBeTruthy();
    const historyFrame = (await rawFramesFromPage(page)).find((frame) =>
      (frame as { type?: string }).type === 'history') as { messages: Array<{ id: string; timestamp: string }> };
    for (const expected of historical) {
      expect(historyFrame.messages).toContainEqual(expect.objectContaining({ id: expected.id, timestamp: expected.browserTime }));
    }
    expect(pageErrors).toEqual([]);
  } finally {
    first.socket.destroy();
    second?.socket.destroy();
    await alice.close();
  }
});

test('channel commit fans out and survives a real browser reconnect', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const bobPage = await bob.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(alicePage, 'alice-channel', pageErrors);
  captureSocketDiagnostics(bobPage, 'bob-channel', pageErrors);
  await Promise.all([alicePage.goto('/'), bobPage.goto('/')]);
  await Promise.all([openGeneral(alicePage), openGeneral(bobPage)]);

  const composer = alicePage.getByPlaceholder('Message #general');
  await composer.fill('channel message across a real socket');
  await composer.press('Enter');
  await expect(alicePage.getByText('channel message across a real socket')).toBeVisible();
  await expect(bobPage.getByText('channel message across a real socket')).toBeVisible();

  const aliceMessage = alicePage.getByLabel('Message from alice').filter({ hasText: 'channel message across a real socket' });
  await aliceMessage.focus();
  await aliceMessage.getByTitle('Edit').click();
  const edit = alicePage.getByLabel('Message from alice').locator('input');
  await edit.fill('edited channel message');
  await edit.press('Enter');
  await expect(bobPage.getByLabel('Message from alice').filter({ hasText: 'edited channel message' })).toBeVisible();
  const aliceEditedMessage = alicePage.getByLabel('Message from alice').filter({ hasText: 'edited channel message' });

  const bobCopy = bobPage.getByLabel('Message from alice').filter({ hasText: 'edited channel message' });
  await bobCopy.focus();
  await bobCopy.getByTitle('React').click();
  await expect(bobCopy.getByRole('button', { name: 'Remove 👍 reaction, 1' })).toBeVisible();
  await expect(aliceEditedMessage.getByRole('button', { name: 'Add 👍 reaction, 1' })).toBeVisible();

  await aliceEditedMessage.focus();
  await aliceEditedMessage.getByTitle('Pin Message').click();
  await aliceEditedMessage.focus();
  await aliceEditedMessage.getByTitle('Bookmark').click();
  await alicePage.getByTitle('Pinned messages').click();
  await expect(alicePage.getByText(/Pinned by/)).toBeVisible();
  await alicePage.getByRole('button', { name: 'Close pinned messages' }).click();
  await alicePage.getByTitle('Bookmarks').click();
  await expect(alicePage.getByText(/Saved on/)).toBeVisible();
  await alicePage.getByRole('button', { name: 'Close bookmarks' }).click();

  await bobCopy.focus();
  await bobCopy.getByTitle('Reply').click();
  const bobComposer = bobPage.getByPlaceholder('Message #general');
  await bobComposer.fill('reply through the canonical conversation');
  await bobComposer.press('Enter');
  await expect(alicePage.getByText('reply through the canonical conversation')).toBeVisible();

  await alicePage.locator('input[type=file]').setInputFiles({
    name: 'evidence.txt', mimeType: 'text/plain', buffer: Buffer.from('private attachment evidence'),
  });
  await expect(alicePage.getByText('evidence.txt')).toBeVisible();
  await alicePage.getByRole('button', { name: 'Send message' }).click();
  await expect(bobPage.getByText('evidence.txt')).toBeVisible();

  await composer.fill('unfurl failure must not block https://example.invalid/concord-preview');
  await composer.press('Enter');
  await expect(bobPage.getByText('unfurl failure must not block')).toBeVisible();

  await alicePage.getByTitle('Search messages').click();
  await alicePage.getByPlaceholder('Search messages...').fill('"edited channel message"');
  await alicePage.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(alicePage.getByText('1 result')).toBeVisible();
  await expect.poll(() => alicePage.evaluate(async () => {
    const response = await fetch('/api/search?server_id=browser-server&q=edited&continuation=invalid');
    return { status: response.status, body: await response.json() };
  })).toMatchObject({ status: 400, body: { code: 'INVALID_CONTINUATION' } });
  await alicePage.getByTitle('Search messages').click();

  await bobPage.reload();
  await openGeneral(bobPage);
  await expect(bobPage.getByLabel('Message from alice').filter({ hasText: 'edited channel message' })).toBeVisible();
  await expect(bobPage.getByText('reply through the canonical conversation')).toBeVisible();
  await expect(bobPage.getByText('evidence.txt')).toBeVisible();
  await expect.poll(() => bobPage.evaluate(async () => {
    const { useChatStore } = await import('/src/stores/chatStore.ts');
    return Object.values(useChatStore.getState().messages).flat()
      .find((message) => message.attachments?.some((attachment) => attachment.filename === 'evidence.txt'))?.content;
  })).toBe('');

  await aliceEditedMessage.focus();
  await aliceEditedMessage.getByTitle('Delete').click();
  await expect(aliceEditedMessage).toBeHidden();
  await expect(bobCopy).toBeHidden();
  await expect(alicePage.getByLabel('Message from bob').filter({ hasText: 'reply through the canonical conversation' })).not.toContainText('edited channel message');
  await expect(bobPage.getByLabel('Message from bob').filter({ hasText: 'reply through the canonical conversation' })).not.toContainText('edited channel message');
  await alicePage.getByTitle('Pinned messages').click();
  await expect(alicePage.getByText('[deleted]')).toBeVisible();
  await expect(alicePage.getByText('edited channel message')).toBeHidden();
  await alicePage.getByRole('button', { name: 'Close pinned messages' }).click();
  await alicePage.getByTitle('Bookmarks').click();
  await expect(alicePage.getByText('[deleted]')).toBeVisible();
  await expect(alicePage.getByText('edited channel message')).toBeHidden();
  await alicePage.getByRole('button', { name: 'Close bookmarks' }).click();
  await alicePage.getByTitle('Search messages').click();
  await alicePage.getByPlaceholder('Search messages...').fill('"edited channel message"');
  await alicePage.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(alicePage.getByText('0 results')).toBeVisible();
  await alicePage.getByTitle('Search messages').click();
  await composer.fill('x'.repeat(4001));
  await composer.press('Enter');
  await expect(alicePage.getByText('Message too long (max 4000 characters)').first()).toBeVisible();
  await expect(alicePage.getByRole('button', { name: 'Retry' })).toBeVisible();
  await bobPage.reload();
  await openGeneral(bobPage);
  await expect(bobPage.getByText('edited channel message')).toBeHidden();
  await expect(bobPage.getByLabel('Message from bob').filter({ hasText: 'reply through the canonical conversation' })).not.toContainText('edited channel message');
  expect(pageErrors).toEqual([]);
  await Promise.all([alice.close(), bob.close()]);
});

test('owner creates a forum and manages its tags through the real server', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  await alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await alice.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'alice-forum', pageErrors);
  await page.goto('/');

  await page.getByTitle('Server Settings').click();
  await page.getByRole('button', { name: 'Channels', exact: true }).click();
  await page.getByPlaceholder('New channel name').fill('help-forum');
  await page.getByLabel('Channel type').selectOption('forum');
  await page.getByRole('button', { name: 'Create', exact: true }).click();

  await expect(page.getByRole('button', { name: '# help-forum' })).toBeVisible();
  await expect(page.getByText('Forum', { exact: true })).toBeVisible();
  await page.getByLabel('New tag name for #help-forum').fill('Solved');
  await page.getByLabel('New tag emoji for #help-forum').fill('✅');
  await page.getByRole('button', { name: 'Add tag' }).click();

  const tagName = page.getByLabel('Tag name for Solved');
  await expect(tagName).toHaveValue('Solved');
  await expect(page.getByLabel('Tag emoji for Solved')).toHaveValue('✅');
  await tagName.fill('Resolved');
  await tagName.blur();
  await expect(page.getByLabel('Tag name for Resolved')).toHaveValue('Resolved');
  await page.getByLabel('Tag name for Resolved').locator('..').getByText('Moderated').click();
  await expect(page.getByLabel('Tag name for Resolved').locator('..').getByRole('checkbox')).toBeChecked();
  await page.getByLabel('Tag name for Resolved').locator('..').getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByLabel('Tag name for Resolved')).toHaveCount(0);

  expect(pageErrors).toEqual([]);
  await alice.close();
});

test('owner times out a member and reads the committed audit record', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  await alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await alice.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'alice-moderation', pageErrors);
  await page.goto('/');

  await page.getByTitle('Moderation').click();
  await page.getByPlaceholder('User ID').fill('browser-bob');
  await page.getByPlaceholder('Optional reason').fill('journey timeout');
  await page.getByRole('button', { name: 'Timeout', exact: true }).click();
  await page.getByRole('button', { name: 'Audit Log' }).click();

  await expect(page.getByText('Timed out', { exact: true })).toBeVisible();
  await expect(page.getByText('Target: browser-bob', { exact: true })).toBeVisible();
  await expect(page.getByText('Reason: journey timeout', { exact: true })).toBeVisible();
  expect(pageErrors).toEqual([]);
  await alice.close();
});

test('owner snapshots and instantiates a server template through the real server', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  await alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await alice.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'alice-template', pageErrors);
  await page.goto('/');

  await page.getByTitle('Community').click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Create Template', exact: true }).click();
  await page.getByPlaceholder('My Server Template').fill('Fixture template');
  await page.getByPlaceholder("What's this template for?").fill('Authenticated journey');
  await page.getByRole('button', { name: 'Create Template', exact: true }).click();

  await expect(page.getByText('Fixture template', { exact: true })).toBeVisible();
  await page.getByLabel('New server name for Fixture template').fill('Template copy');
  await page.getByRole('button', { name: 'Create Server', exact: true }).click();
  await expect(page.getByTitle('Template copy')).toBeVisible();
  expect(pageErrors).toEqual([]);
  await alice.close();
});

test('owner manages multiple servers categories and immediate private visibility', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const bobPage = await bob.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(alicePage, 'alice-organization', pageErrors);
  captureSocketDiagnostics(bobPage, 'bob-private-visibility', pageErrors);
  await Promise.all([alicePage.goto('/'), bobPage.goto('/')]);

  await alicePage.getByTitle('Server Settings').click();
  await alicePage.getByRole('button', { name: 'Categories', exact: true }).click();
  await alicePage.getByPlaceholder('New category name').fill('Private work');
  await alicePage.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(alicePage.getByText('Private work', { exact: true }).last()).toBeVisible();
  await alicePage.getByRole('button', { name: 'Channels', exact: true }).click();
  await alicePage.getByPlaceholder('New channel name').fill('owners-room');
  await alicePage.getByLabel('Channel category').selectOption({ label: 'Private work' });
  await alicePage.getByText('Private', { exact: true }).click();
  await alicePage.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(alicePage.getByText('owners-room', { exact: true }).last()).toBeVisible();
  await expect(alicePage.getByRole('button', { name: 'owners-room', exact: true })).toBeVisible();

  await bobPage.reload();
  await bobPage.getByTitle('Browser fixture').click();
  await expect(bobPage.getByRole('button', { name: 'owners-room', exact: true })).toHaveCount(0);

  await alicePage.getByTitle('Close settings').click();
  await alicePage.getByTitle('Create a server').click();
  await alicePage.getByPlaceholder('My Awesome Server').fill('Second fixture');
  await alicePage.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(alicePage.getByTitle('Second fixture')).toBeVisible();
  await alicePage.getByTitle('Second fixture').click();
  await expect(alicePage.getByRole('button', { name: 'general' })).toBeVisible();
  await alicePage.getByTitle('Server Settings').click();
  await alicePage.getByRole('button', { name: 'Delete Server', exact: true }).click();
  await expect(alicePage.getByTitle('Second fixture')).toHaveCount(0);
  await expect(alicePage.getByTitle('Browser fixture')).toBeVisible();
  await expect(alicePage.getByRole('button', { name: /general/ })).toBeVisible();
  await expect(alicePage.getByPlaceholder('Message #general')).toBeEditable();

  expect(pageErrors).toEqual([]);
  await Promise.all([alice.close(), bob.close()]);
});

test('a member edits profile presence and server identity through settings', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const bobPage = await bob.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(alicePage, 'alice-profile', pageErrors);
  captureSocketDiagnostics(bobPage, 'bob-presence', pageErrors);
  await Promise.all([alicePage.goto('/'), bobPage.goto('/')]);
  await Promise.all([openGeneral(alicePage), openGeneral(bobPage)]);
  const bobMembers = bobPage.getByLabel('Members');
  await bobMembers.getByRole('button', { name: /alice/i }).click();

  await alicePage.getByRole('button', { name: 'U Settings', exact: true }).click();
  await alicePage.getByLabel('Pronouns').fill('she/her');
  await alicePage.getByLabel('Bio').fill('Profile edited through the real settings journey');
  await alicePage.getByRole('button', { name: 'Save profile' }).click();
  await expect.poll(() => alicePage.evaluate(async () => {
    const response = await fetch('/api/users/browser-alice/profile');
    return response.json();
  })).toMatchObject({ pronouns: 'she/her', bio: 'Profile edited through the real settings journey' });
  await expect(bobPage.getByText('she/her', { exact: true })).toBeVisible();
  await expect(bobPage.getByText('Profile edited through the real settings journey', { exact: true })).toBeVisible();

  await alicePage.getByLabel('Presence status').selectOption('idle');
  await alicePage.getByLabel('Status emoji').fill('🌙');
  await alicePage.getByLabel('Custom status').fill('Writing tests');
  await alicePage.getByRole('button', { name: 'Update presence' }).click();
  await expect(bobPage.getByRole('dialog', { name: 'alice profile' }).getByText('Writing tests')).toBeVisible();
  await bobPage.keyboard.press('Escape');

  await alicePage.getByLabel('Server nickname').fill('Alice Server Alias');
  await alicePage.getByRole('button', { name: 'Save server nickname' }).click();
  await expect(bobMembers.getByRole('button', { name: /Alice Server Alias/ })).toBeVisible();

  await alicePage.getByLabel('Upload server avatar').setInputFiles({
    name: 'server-avatar.png',
    mimeType: 'image/png',
    buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', 'base64'),
  });
  await expect(bobMembers.getByRole('button', { name: /Alice Server Alias/ }).locator('img')).toHaveAttribute('src', /\/api\/uploads\//);

  await alicePage.getByLabel('Server nickname').fill('');
  await alicePage.getByRole('button', { name: 'Save server nickname' }).click();
  await expect(bobMembers.getByRole('button', { name: /Alice Server Alias/ })).toHaveCount(0);
  await expect(bobMembers.getByRole('button', { name: /alice/i })).toBeVisible();
  await bobPage.reload();
  await openGeneral(bobPage);
  await expect(bobPage.getByLabel('Members').getByRole('button', { name: /Alice Server Alias/ })).toHaveCount(0);
  await expect(bobPage.getByLabel('Members').getByRole('button', { name: /alice/i }).locator('img')).toHaveAttribute('src', /\/api\/uploads\//);
  expect(pageErrors).toEqual([]);
  await Promise.all([alice.close(), bob.close()]);
});

test('owner manages colored role permissions and member assignment', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const page = await alice.newPage();
  const bobPage = await bob.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'alice-roles', pageErrors);
  captureSocketDiagnostics(bobPage, 'bob-roles', pageErrors);
  await Promise.all([page.goto('/'), bobPage.goto('/')]);
  await Promise.all([openGeneral(page), openGeneral(bobPage)]);

  const bobName = () => bobPage.getByLabel('Members').getByRole('button', { name: /bob/i }).locator('span.truncate.text-sm');
  const expectBobColor = async (expected: string) => {
    await expect.poll(() => bobName().evaluate((element) => getComputedStyle(element).color)).toBe(expected);
  };

  await page.getByTitle('Server Settings').click();
  await page.getByRole('button', { name: 'Roles', exact: true }).click();
  await page.getByPlaceholder('New role name').fill('Journey role');
  await page.getByLabel('New role color').fill('#7c3aed');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  const roleCard = page.getByLabel('Role Journey role');
  await expect(roleCard.getByText('Journey role', { exact: true })).toBeVisible();
  await roleCard.getByRole('button', { name: 'Edit', exact: true }).click();
  await roleCard.getByText('Manage Channels', { exact: true }).click();
  await roleCard.locator('input[type="text"]').fill('Purple steward');
  await roleCard.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Purple steward', { exact: true }).last()).toBeVisible();

  await page.getByPlaceholder('Member user ID').fill('browser-bob');
  await page.getByLabel('Assign Purple steward for browser-bob').click();
  await expect(page.getByLabel('Remove Purple steward for browser-bob')).toBeChecked();
  await expectBobColor('rgb(124, 58, 237)');

  await bobPage.reload();
  await openGeneral(bobPage);
  await expectBobColor('rgb(124, 58, 237)');

  await page.getByRole('button', { name: 'Channels', exact: true }).click();
  const generalPermissions = page.getByLabel('Channel general');
  await generalPermissions.getByRole('button', { name: 'Permissions', exact: true }).click();
  await generalPermissions.getByLabel('Override role').selectOption({ label: 'Purple steward' });
  await generalPermissions.getByLabel('View channel for Purple steward').selectOption('deny');
  await generalPermissions.getByRole('button', { name: 'Save permissions' }).click();
  await expect(generalPermissions.getByRole('button', { name: 'Reset to inherited' })).toBeVisible();

  await bobPage.reload();
  await bobPage.getByTitle('Browser fixture').click();
  await expect(bobPage.getByRole('button', { name: 'general', exact: true })).toHaveCount(0);

  await generalPermissions.getByRole('button', { name: 'Reset to inherited' }).click();
  await expect(generalPermissions.getByRole('button', { name: 'Reset to inherited' })).toHaveCount(0);
  await bobPage.reload();
  await openGeneral(bobPage);
  await expectBobColor('rgb(124, 58, 237)');

  await page.getByRole('button', { name: 'Roles', exact: true }).click();
  await page.getByPlaceholder('Member user ID').fill('browser-bob');

  await page.getByLabel('Remove Purple steward for browser-bob').click();
  await expect(page.getByLabel('Assign Purple steward for browser-bob')).not.toBeChecked();
  await expect.poll(() => bobName().evaluate((element) => getComputedStyle(element).color))
    .not.toBe('rgb(124, 58, 237)');

  await page.getByLabel('Assign Purple steward for browser-bob').click();
  await expectBobColor('rgb(124, 58, 237)');
  const updatedCard = page.getByLabel('Role Purple steward');
  await updatedCard.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('Purple steward', { exact: true })).toHaveCount(0);
  await expect.poll(() => bobName().evaluate((element) => getComputedStyle(element).color))
    .not.toBe('rgb(124, 58, 237)');

  expect(pageErrors).toEqual([]);
  await Promise.all([alice.close(), bob.close()]);
});

test('private thread remains scoped and archive state survives restart', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const page = await alice.newPage();
  const bobPage = await bob.newPage();
  await Promise.all([page.goto('/'), bobPage.goto('/')]);
  await Promise.all([openGeneral(page), openGeneral(bobPage)]);

  await page.getByPlaceholder('Message #general').fill('private thread parent');
  await page.getByRole('button', { name: 'Send message' }).click();
  const parent = page.locator('[data-message-id]').filter({ hasText: 'private thread parent' }).last();
  await parent.hover();
  await parent.getByTitle('Create Thread').click();
  const dialog = page.getByRole('dialog', { name: 'Create thread' });
  await dialog.getByLabel('Thread name').fill('private-planning');
  await dialog.getByLabel('Private thread').check();
  await dialog.getByRole('button', { name: 'Create thread' }).click();

  const threadButton = page.getByRole('region', { name: 'Threads' }).getByRole('button', { name: /private-planning/ });
  await expect(threadButton).toBeVisible();
  await expect(bobPage.getByRole('region', { name: 'Threads' }).getByRole('button', { name: /private-planning/ })).toHaveCount(0);
  await threadButton.click();
  await expect(page.getByText('Private', { exact: true })).toBeVisible();
  await page.getByTitle('Archive thread').click();
  await expect(page.getByTitle('Unarchive thread')).toBeVisible();

  const restartRequest = process.env.CONCORD_RESTART_REQUEST!;
  const restartAck = process.env.CONCORD_RESTART_ACK!;
  if (existsSync(restartAck)) unlinkSync(restartAck);
  writeFileSync(restartRequest, `${Date.now()}\n`, { flag: 'wx' });
  await expect.poll(() => existsSync(restartAck), { timeout: 15_000 }).toBe(true);
  await Promise.all([page.reload(), bobPage.reload()]);
  await Promise.all([openGeneral(page), openGeneral(bobPage)]);
  await expect(page.getByRole('region', { name: 'Threads' }).getByRole('button', { name: /private-planning/ })).toBeVisible();
  await expect(bobPage.getByRole('region', { name: 'Threads' }).getByRole('button', { name: /private-planning/ })).toHaveCount(0);
  await page.getByRole('region', { name: 'Threads' }).getByRole('button', { name: /private-planning/ }).click();
  await expect(page.getByTitle('Unarchive thread')).toBeVisible();
  await page.getByTitle('Unarchive thread').click();
  await expect(page.getByTitle('Archive thread')).toBeVisible();

  await Promise.all([alice.close(), bob.close()]);
});

test('owner manages local emoji and stickers through the browser with policy and revocation', async ({ browser, baseURL }) => {
  const context = await browser.newContext();
  await context.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await context.newPage();
  const pageErrors: string[] = [];
  captureSocketDiagnostics(page, 'managed-media', pageErrors);
  await page.goto('/');

  await page.getByTitle('Create a server').click();
  await page.getByPlaceholder('My Awesome Server').fill('Media policy target');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByTitle('Media policy target')).toBeVisible();
  const targetServerId = await page.evaluate(async () => {
    const servers = await fetch('/api/servers').then((response) => response.json()) as Array<{ id: string; name: string }>;
    const target = servers.find((server) => server.name === 'Media policy target');
    if (!target) throw new Error('created media policy target was not returned by the server');
    return target.id;
  });

  await openGeneral(page);
  await page.getByTitle('Server Settings').click();
  const settings = page.getByRole('dialog', { name: 'Browser fixture Settings' });
  const pixel = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', 'base64');

  await settings.getByRole('button', { name: 'Emoji', exact: true }).click();
  await settings.locator('input[type="file"]').setInputFiles({
    name: 'journey-emoji.png', mimeType: 'image/png', buffer: pixel,
  });
  await settings.getByPlaceholder('emoji_name').fill('journey_wave');
  await settings.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(settings.getByRole('img', { name: 'journey_wave' })).toBeVisible();
  const emoji = await page.evaluate(async () => {
    const emoji = await fetch('/api/servers/browser-server/emoji').then((response) => response.json()) as Array<{ id: string; name: string; image_url: string }>;
    const created = emoji.find((candidate) => candidate.name === 'journey_wave');
    if (!created) throw new Error('managed emoji was not committed');
    return created;
  });

  await settings.getByRole('button', { name: 'Stickers', exact: true }).click();
  await settings.locator('input[type="file"]').setInputFiles({
    name: 'journey-sticker.png', mimeType: 'image/png', buffer: pixel,
  });
  await settings.getByPlaceholder('sticker_name').fill('journey_sticker');
  await settings.getByPlaceholder('Description (optional)').fill('Browser managed sticker');
  await settings.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(settings.getByRole('img', { name: 'journey_sticker' })).toBeVisible();
  const sticker = await page.evaluate(async () => {
    const stickers = await fetch('/api/servers/browser-server/stickers').then((response) => response.json()) as Array<{ id: string; name: string; image_url: string }>;
    const created = stickers.find((candidate) => candidate.name === 'journey_sticker');
    if (!created) throw new Error('managed sticker was not committed');
    return created;
  });

  await settings.getByTitle('Close settings').click();
  const composer = page.getByPlaceholder('Message #general');
  await composer.fill('Managed media :journey_wave: [sticker:journey_sticker]');
  await page.getByRole('button', { name: 'Send message' }).click();
  const rendered = page.locator('[data-message-id]').filter({ hasText: 'Managed media' }).last();
  await expect(rendered.getByRole('img', { name: ':journey_wave:' })).toBeVisible();
  await expect(rendered.getByRole('img', { name: '[sticker:journey_sticker]' })).toBeVisible();
  expect(await page.evaluate((url) => fetch(url).then((response) => response.status), emoji.image_url)).toBe(200);
  expect(await page.evaluate((url) => fetch(url).then((response) => response.status), sticker.image_url)).toBe(200);

  const sharedNames = await page.evaluate((target) => fetch(`/api/users/me/emoji?target_server_id=${encodeURIComponent(target)}`).then(async (response) => ({
    status: response.status,
    names: (await response.json() as Array<{ name: string }>).map((item) => item.name),
  })), targetServerId);
  expect(sharedNames).toEqual(expect.objectContaining({ status: 200, names: expect.arrayContaining(['journey_wave']) }));
  expect(await page.evaluate((target) => fetch(`/api/servers/${encodeURIComponent(target)}/emoji-settings`, {
    method: 'PATCH', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ allow_external_emoji: false, shareable_emoji: true }),
  }).then((response) => response.status), targetServerId)).toBe(204);
  expect(await page.evaluate((target) => fetch(`/api/users/me/emoji?target_server_id=${encodeURIComponent(target)}`).then(async (response) => ({
    status: response.status,
    names: (await response.json() as Array<{ name: string }>).map((item) => item.name),
  })), targetServerId)).toEqual({ status: 200, names: [] });

  await page.getByTitle('Server Settings').click();
  await settings.getByRole('button', { name: 'Emoji', exact: true }).click();
  await settings.getByRole('button', { name: 'Delete emoji journey_wave' }).click();
  await expect(settings.getByRole('img', { name: 'journey_wave' })).toHaveCount(0);
  await settings.getByRole('button', { name: 'Stickers', exact: true }).click();
  await settings.getByRole('button', { name: 'Delete sticker journey_sticker' }).click();
  await expect(settings.getByRole('img', { name: 'journey_sticker' })).toHaveCount(0);
  await settings.getByTitle('Close settings').click();

  await expect(rendered.getByRole('img', { name: ':journey_wave:' })).toHaveCount(0);
  await expect(rendered.getByRole('img', { name: '[sticker:journey_sticker]' })).toHaveCount(0);
  expect(await page.evaluate((url) => fetch(url).then((response) => response.status), emoji.image_url)).toBe(404);
  expect(await page.evaluate((url) => fetch(url).then((response) => response.status), sticker.image_url)).toBe(404);
  await page.getByTitle('Media policy target').click();
  await page.getByTitle('Server Settings').click();
  await page.getByRole('button', { name: 'Delete Server', exact: true }).click();
  await expect(page.getByTitle('Media policy target')).toHaveCount(0);
  expect(pageErrors).toEqual([]);
  await context.close();
});
