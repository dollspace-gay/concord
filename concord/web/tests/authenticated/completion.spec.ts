import { expect, test, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { createHash, createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import WebSocket from 'ws';

const sessions = JSON.parse(readFileSync(process.env.CONCORD_AUTH_SESSIONS_FILE!, 'utf8')) as Record<
  'alice' | 'bob' | 'wrong_bot', string
>;

test.describe.configure({ timeout: 90_000 });

async function openGeneral(page: Page) {
  await page.getByTitle('Browser fixture').click();
  await page.getByRole('button', { name: 'general' }).click();
}

async function openIntegrations(page: Page) {
  await page.getByRole('button', { name: 'Integrations' }).click();
  const dialog = page.getByRole('dialog', { name: 'Integrations' });
  await expect(dialog).toBeVisible();
  return dialog;
}

function integrationCard(dialog: Locator, name: string) {
  return dialog.getByText(name, { exact: true }).locator(
    'xpath=ancestor::div[contains(concat(" ", normalize-space(@class), " "), " bg-bg-secondary ")][1]',
  );
}

async function botSocket(baseURL: string, token: string) {
  const frames: unknown[] = [];
  const socket = new WebSocket(baseURL.replace(/^http/, 'ws') + '/ws', {
    headers: { Authorization: `Bearer ${token}`, Origin: baseURL },
  });
  socket.on('message', (data) => {
    try { frames.push(JSON.parse(data.toString())); } catch { /* Diagnostic frames are not protocol events. */ }
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

const isErrorFrame = (frame: unknown) => ['error', 'command_error'].includes(
  (frame as { type?: string }).type ?? '',
);

function receiverRecords() {
  const path = process.env.CONCORD_WEBHOOK_RECEIVER_LOG!;
  try {
    return readFileSync(path, 'utf8').trim().split('\n').filter(Boolean).map((line) => JSON.parse(line) as {
      path: string;
      attempt: number;
      status: number;
      headers: Record<string, string>;
      body: string;
    });
  } catch {
    return [];
  }
}

test('webhook UI completes one-time auth, duplicate-safe ingress, signed delivery retry, and revocation', async ({ browser, baseURL }) => {
  const context = await browser.newContext();
  await context.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  await context.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, 'clipboard', {
      configurable: true,
      get: () => ({
        writeText: (value: string) => {
          (window as typeof window & { copiedIntegrationCredential?: string }).copiedIntegrationCredential = value;
          return Promise.resolve();
        },
      }),
    });
  });
  const page = await context.newPage();
  await page.goto('/');
  await openGeneral(page);
  const dialog = await openIntegrations(page);

  const outgoingName = `completion-outgoing-${Date.now()}`;
  await dialog.getByRole('button', { name: 'Create Webhook' }).click();
  await dialog.getByPlaceholder('Webhook name').fill(outgoingName);
  await dialog.locator('select').nth(0).selectOption('browser-general');
  await dialog.locator('select').nth(1).selectOption('outgoing');
  await dialog.getByPlaceholder('Outgoing URL').fill(
    `http://webhook.fixture:${process.env.CONCORD_WEBHOOK_RECEIVER_PORT}/fail-once`,
  );
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(dialog.getByText('Webhook created.')).toBeVisible();
  const outgoing = integrationCard(dialog, outgoingName);
  const signingSecret = (await outgoing.locator('code').textContent())!;
  expect(signingSecret).toMatch(/^[0-9a-f-]+\.[0-9a-f-]+$/);

  await outgoing.getByRole('button', { name: 'Test' }).click();
  await expect.poll(async () => {
    const refresh = outgoing.getByRole('button', { name: /Refresh status|Loading/ });
    if (await refresh.isEnabled()) await refresh.click();
    return outgoing.textContent();
  }, { timeout: 12_000 }).toContain('failed');
  await expect.poll(() => receiverRecords().filter((record) => record.path === '/fail-once').length).toBe(1);
  const rejected = receiverRecords().find((record) => record.path === '/fail-once')!;
  expect(rejected.status).toBe(400);
  expect(rejected.headers['x-concord-delivery']).toBeTruthy();
  expect(rejected.headers['x-concord-timestamp']).toMatch(/^\d+$/);
  expect(rejected.headers['x-concord-signature-256']).toBe(
    `sha256=${createHmac('sha256', signingSecret).update(`${rejected.headers['x-concord-timestamp']}.${rejected.body}`).digest('hex')}`,
  );
  expect(JSON.parse(rejected.body)).toMatchObject({
    event_type: 'webhook_test',
    server_id: 'browser-server',
    channel_id: 'browser-general',
  });

  await outgoing.getByRole('button', { name: 'Retry' }).click();
  await expect.poll(async () => {
    const refresh = outgoing.getByRole('button', { name: /Refresh status|Loading/ });
    if (await refresh.isEnabled()) await refresh.click();
    return outgoing.textContent();
  }, { timeout: 12_000 }).toContain('delivered');
  await expect.poll(() => receiverRecords().filter((record) => record.path === '/fail-once').length).toBe(2);
  const delivered = receiverRecords().filter((record) => record.path === '/fail-once')[1];
  expect(delivered.status).toBe(204);
  expect(delivered.headers['x-concord-delivery']).toBe(rejected.headers['x-concord-delivery']);
  expect(delivered.headers['x-concord-signature-256']).toBe(
    `sha256=${createHmac('sha256', signingSecret).update(`${delivered.headers['x-concord-timestamp']}.${delivered.body}`).digest('hex')}`,
  );
  await outgoing.getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.getByText(outgoingName, { exact: true })).toHaveCount(0);

  const incomingName = `completion-incoming-${Date.now()}`;
  await dialog.getByRole('button', { name: 'Create Webhook' }).click();
  await dialog.getByPlaceholder('Webhook name').fill(incomingName);
  await dialog.locator('select').nth(0).selectOption('browser-general');
  await dialog.locator('select').nth(1).selectOption('incoming');
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(dialog.getByText('Webhook created.')).toBeVisible();
  const incoming = integrationCard(dialog, incomingName);
  await incoming.getByRole('button', { name: 'Copy webhook URL' }).click();
  const incomingUrl = await page.evaluate(() => (
    window as typeof window & { copiedIntegrationCredential?: string }
  ).copiedIntegrationCredential);
  expect(incomingUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/api\/webhooks\//);

  const content = `duplicate-safe webhook ${Date.now()}`;
  const deliverIncoming = () => page.evaluate(async ({ url, body }) => {
    const response = await fetch(url!, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    return { status: response.status, body: await response.text() };
  }, { url: incomingUrl, body: { content, idempotency_key: 'completion-stable-key' } });
  const first = await deliverIncoming();
  expect([200, 201]).toContain(first.status);
  const duplicate = await deliverIncoming();
  expect([200, 201]).toContain(duplicate.status);
  await expect(page.getByText(content, { exact: true })).toHaveCount(1);

  await incoming.getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.getByText(incomingName, { exact: true })).toHaveCount(0);
  expect([400, 401, 404]).toContain((await deliverIncoming()).status);
  await context.close();
});

test('bot UI token lifecycle drives correlated slash and component outcomes over real sockets', async ({ browser, baseURL }) => {
  const alice = await browser.newContext();
  const bob = await browser.newContext();
  await Promise.all([
    alice.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]),
    bob.addCookies([{ name: 'concord_session', value: sessions.bob, url: baseURL! }]),
  ]);
  const alicePage = await alice.newPage();
  const bobPage = await bob.newPage();
  await Promise.all([alicePage.goto('/'), bobPage.goto('/')]);
  await Promise.all([openGeneral(alicePage), openGeneral(bobPage)]);
  const dialog = await openIntegrations(alicePage);
  await dialog.getByRole('button', { name: 'Bots' }).click();

  const botName = `completion-bot-${Date.now()}`;
  await dialog.getByRole('button', { name: 'Create Bot' }).click();
  await dialog.getByPlaceholder('Bot username').fill(botName);
  await dialog.getByRole('button', { name: 'Create Bot', exact: true }).click();
  await expect(dialog.getByText('Bot account created.')).toBeVisible();
  await dialog.getByRole('button', { name: 'Install on server' }).click();
  await expect(dialog.getByText('Bot installed on server.')).toBeVisible();
  await dialog.getByRole('button', { name: 'I saved it' }).click();

  const tokenName = `completion-token-${Date.now()}`;
  await dialog.getByLabel('Token name').fill(tokenName);
  await dialog.getByLabel('Token scopes').fill('bot commands messages');
  await dialog.getByRole('button', { name: 'Create token' }).click();
  await expect.poll(async () => (
    await dialog.getByText('Bot token created.').count()
      + await dialog.getByRole('alert').count()
  )).toBeGreaterThan(0);
  if (await dialog.getByRole('alert').count()) {
    await expect(dialog.getByLabel('Token name')).toHaveValue(tokenName);
    await expect(dialog.getByLabel('Token scopes')).toHaveValue('bot commands messages');
    await dialog.getByRole('button', { name: 'Create token' }).click();
  }
  await expect(dialog.getByText('Bot token created.')).toBeVisible();
  const credential = dialog.locator('[role="status"]').filter({ hasText: 'Copy this bot token now' });
  const botToken = (await credential.locator('code').textContent())!;
  expect(botToken).toMatch(/^cc_bot_/);

  let bot = await botSocket(baseURL!, botToken);
  const commandName = `completion${Date.now().toString().slice(-6)}`;
  bot.send({
    type: 'register_slash_command', server_id: 'browser-server', name: commandName,
    description: 'Completion journey command', options_json: '[]',
  });
  await expect.poll(() => bot.frames.filter(isErrorFrame)).toEqual([]);
  const composer = alicePage.getByPlaceholder('Message #general');
  await composer.fill(`/${commandName}`);
  await expect(alicePage.getByRole('option', { name: new RegExp(commandName) })).toBeVisible();

  await dialog.getByRole('button', { name: 'Remove from server' }).click();
  await expect(dialog.getByText('Bot removed from server.')).toBeVisible();
  bot.socket.close();
  await dialog.getByRole('button', { name: '×' }).click();
  await composer.press('Enter');
  await expect(alicePage.getByRole('alert')).toContainText(/bot|install|command/i);
  await expect(composer).toHaveValue(`/${commandName}`);

  const retryDialog = await openIntegrations(alicePage);
  await retryDialog.getByRole('button', { name: 'Bots' }).click();
  await retryDialog.getByLabel('Bot account').selectOption({ label: botName });
  await retryDialog.getByRole('button', { name: 'Install on server' }).click();
  await expect(retryDialog.getByText('Bot installed on server.')).toBeVisible();
  await retryDialog.getByRole('button', { name: '×' }).click();
  bot = await botSocket(baseURL!, botToken);
  await composer.press('Enter');
  await expect(composer).toHaveValue('');
  await expect.poll(() => bot.frames.filter((frame) => (
    frame as { type?: string }
  ).type === 'interaction_create').length).toBe(1);
  const slash = bot.frames.find((frame) => (frame as { type?: string }).type === 'interaction_create') as {
    interaction: { id: string };
  };

  const wrong = await botSocket(baseURL!, sessions.wrong_bot);
  wrong.send({ type: 'respond_to_interaction', interaction_id: slash.interaction.id, content: 'wrong bot' });
  await expect.poll(() => wrong.frames.filter(isErrorFrame).length).toBe(1);

  const errorsBeforeMalformed = bot.frames.filter(isErrorFrame).length;
  bot.send({
    type: 'respond_to_interaction', interaction_id: slash.interaction.id, content: 'malformed', ephemeral: false,
    components_json: JSON.stringify([{ type: 'action_row', components: [
      { type: 'button', custom_id: 'duplicate', label: 'One', style: 'primary' },
      { type: 'button', custom_id: 'duplicate', label: 'Two', style: 'primary' },
    ] }]),
  });
  await expect.poll(() => bot.frames.filter(isErrorFrame).length).toBe(errorsBeforeMalformed + 1);
  await expect(alicePage.getByText('malformed', { exact: true })).toHaveCount(0);

  bot.send({
    type: 'respond_to_interaction', interaction_id: slash.interaction.id,
    content: 'Completion public response', ephemeral: false,
    embeds_json: JSON.stringify([{ title: 'Safe rich response', description: 'Rendered through the browser' }]),
    components_json: JSON.stringify([{ type: 'action_row', components: [
      { type: 'button', custom_id: 'continue', label: 'Continue completion', style: 'primary' },
    ] }]),
  });
  await Promise.all([
    expect(alicePage.getByText('Completion public response')).toBeVisible(),
    expect(bobPage.getByText('Completion public response')).toBeVisible(),
    expect(alicePage.getByText('Safe rich response')).toBeVisible(),
  ]);
  await alicePage.getByRole('button', { name: 'Continue completion' }).click();
  await expect.poll(() => bot.frames.filter((frame) => (
    frame as { interaction?: { interaction_type?: string } }
  ).interaction?.interaction_type === 'button').length).toBe(1);
  const button = bot.frames.find((frame) => (
    frame as { interaction?: { interaction_type?: string } }
  ).interaction?.interaction_type === 'button') as { interaction: { id: string } };

  bot.send({
    type: 'respond_to_interaction', interaction_id: button.interaction.id,
    content: 'Completion private response', ephemeral: true,
    components_json: JSON.stringify([{ type: 'action_row', components: [{
      type: 'select_menu', custom_id: 'private-choice', placeholder: 'Private completion choice',
      min_values: 1, max_values: 1,
      options: [{ label: 'Alpha', value: 'alpha' }, { label: 'Beta', value: 'beta' }],
    }] }]),
  });
  await expect(alicePage.getByText('Completion private response')).toBeVisible();
  await expect(bobPage.getByText('Completion private response')).toHaveCount(0);
  await alicePage.getByRole('combobox', { name: 'Private completion choice' }).selectOption('beta');
  await expect.poll(() => bot.frames.filter((frame) => (
    frame as { interaction?: { interaction_type?: string } }
  ).interaction?.interaction_type === 'select_menu').length).toBe(1);
  const selected = bot.frames.find((frame) => (
    frame as { interaction?: { interaction_type?: string } }
  ).interaction?.interaction_type === 'select_menu') as { interaction: { data: { values: string[] } } };
  expect(selected.interaction.data.values).toEqual(['beta']);

  const errorsBeforeReplay = bot.frames.filter(isErrorFrame).length;
  bot.send({ type: 'respond_to_interaction', interaction_id: button.interaction.id, content: 'replay', ephemeral: true });
  await expect.poll(() => bot.frames.filter(isErrorFrame).length).toBe(errorsBeforeReplay + 1);

  const revokeDialog = await openIntegrations(alicePage);
  await revokeDialog.getByRole('button', { name: 'Bots' }).click();
  await revokeDialog.getByLabel('Bot account').selectOption({ label: botName });
  const tokenRow = revokeDialog.getByText(tokenName, { exact: true }).locator('../..');
  await tokenRow.getByRole('button', { name: 'Revoke' }).click();
  await expect(revokeDialog.getByText('Bot token revoked.')).toBeVisible();
  await Promise.race([
    bot.closed,
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error('revoked bot socket remained open')), 5_000)),
  ]);
  await expect(botSocket(baseURL!, botToken)).rejects.toThrow(/401/);
  expect(wrong.socket.readyState).toBe(WebSocket.OPEN);
  wrong.socket.close();
  await Promise.all([alice.close(), bob.close()]);
});

async function exchangePublicCode(
  request: APIRequestContext,
  baseURL: string,
  clientId: string,
  code: string,
  verifier: string,
) {
  return request.post(`${baseURL}/api/oauth/token`, {
    form: {
      grant_type: 'authorization_code', client_id: clientId, code,
      redirect_uri: 'https://client.example/callback', code_verifier: verifier,
    },
  });
}

test('OAuth app UI completes public PKCE consent, replay, refresh, regrant, scope, and uninstall', async ({ browser, baseURL, request }) => {
  const oauthBaseURL = process.env.CONCORD_BACKEND_URL!;
  const context = await browser.newContext();
  await context.addCookies([{ name: 'concord_session', value: sessions.alice, url: baseURL! }]);
  const page = await context.newPage();
  await page.goto('/');
  await openGeneral(page);
  const dialog = await openIntegrations(page);
  await dialog.getByRole('button', { name: 'OAuth Apps' }).click();

  const appName = `completion-oauth-${Date.now()}`;
  await dialog.getByRole('button', { name: 'Create App' }).click();
  await dialog.getByPlaceholder('App name').fill(appName);
  await dialog.getByPlaceholder('Description').fill('Delegated lifecycle browser fixture');
  await dialog.getByPlaceholder('Redirect URIs (comma separated)').fill('https://client.example/callback');
  await dialog.getByLabel('OAuth client type').selectOption('public');
  await dialog.getByRole('button', { name: 'Create App', exact: true }).click();
  await expect(dialog.getByText('OAuth application created.')).toBeVisible();
  const appCard = integrationCard(dialog, appName);
  await expect(appCard.getByText('Public', { exact: true })).toBeVisible();
  const clientId = (await appCard.locator('code').textContent())!;

  const verifier = 'completion-verifier-abcdefghijklmnopqrstuvwxyz0123456789';
  const challenge = createHash('sha256').update(verifier).digest('base64url');
  const authorizeUrl = (scope = 'identify servers.read', redirect = 'https://client.example/callback') => {
    const url = new URL('/oauth/authorize', baseURL);
    url.search = new URLSearchParams({
      response_type: 'code', client_id: clientId, redirect_uri: redirect, scope,
      state: 'completion-state', code_challenge: challenge, code_challenge_method: 'S256',
      server_id: 'browser-server',
    }).toString();
    return url.toString();
  };
  const unregisteredRedirect = await context.request.get(
    authorizeUrl('identify', 'https://client.example/callback?unregistered=1'),
    { maxRedirects: 0 },
  );
  expect(unregisteredRedirect.status()).toBe(400);

  const consentPage = await context.newPage();
  const authorize = async (scope = 'identify servers.read') => {
    await consentPage.goto(authorizeUrl(scope));
    await expect(consentPage.getByRole('heading', { name: `Authorize ${appName}` })).toBeVisible();
    await expect(consentPage.getByText('Published by alice')).toBeVisible();
    await expect(consentPage.getByText('Access target: server Browser fixture')).toBeVisible();
    await expect(consentPage.getByText(`This app requests: ${scope}`)).toBeVisible();
    await expect(consentPage.getByRole('button', { name: 'Authorize' })).toBeVisible();
    await expect(consentPage.getByRole('button', { name: 'Deny' })).toBeVisible();
    const consentToken = await consentPage.locator('input[name="consent_token"]').inputValue();
    const approval = await context.request.post(`${baseURL}/oauth/authorize`, {
      form: { consent_token: consentToken, decision: 'approve' },
      maxRedirects: 0,
    });
    expect(approval.status()).toBe(303);
    const callback = new URL(approval.headers().location);
    expect(callback.searchParams.get('state')).toBe('completion-state');
    return callback.searchParams.get('code')!;
  };

  const code = await authorize();
  const firstResponse = await exchangePublicCode(request, oauthBaseURL, clientId, code, verifier);
  expect(firstResponse.status()).toBe(200);
  const first = await firstResponse.json() as { access_token: string; refresh_token: string; scope: string };
  expect(first.scope).toBe('identify servers.read');
  const firstIdentity = await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${first.access_token}` },
  });
  expect(firstIdentity.status()).toBe(200);
  expect(await firstIdentity.json()).toMatchObject({ id: 'browser-alice' });
  const servers = await request.get(`${oauthBaseURL}/api/oauth/servers`, {
    headers: { Authorization: `Bearer ${first.access_token}` },
  });
  expect(servers.status()).toBe(200);
  expect(await servers.json()).toEqual([{ id: 'browser-server', name: 'Browser fixture' }]);
  expect((await exchangePublicCode(request, oauthBaseURL, clientId, code, verifier)).status()).toBe(400);
  expect((await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${first.access_token}` },
  })).status()).toBe(200);

  const rotatedResponse = await request.post(`${oauthBaseURL}/api/oauth/token`, {
    form: { grant_type: 'refresh_token', client_id: clientId, refresh_token: first.refresh_token },
  });
  expect(rotatedResponse.status()).toBe(200);
  const rotated = await rotatedResponse.json() as { access_token: string; refresh_token: string };
  expect((await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${first.access_token}` },
  })).status()).toBe(401);
  expect((await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${rotated.access_token}` },
  })).status()).toBe(200);

  const reused = await request.post(`${oauthBaseURL}/api/oauth/token`, {
    form: { grant_type: 'refresh_token', client_id: clientId, refresh_token: first.refresh_token },
  });
  expect(reused.status()).toBe(400);
  expect((await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${rotated.access_token}` },
  })).status()).toBe(401);

  const regrantCode = await authorize('identify');
  const regrantResponse = await exchangePublicCode(request, oauthBaseURL, clientId, regrantCode, verifier);
  expect(regrantResponse.status()).toBe(200);
  const regrant = await regrantResponse.json() as { access_token: string; refresh_token: string; scope: string };
  expect(regrant.scope).toBe('identify');
  expect((await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${regrant.access_token}` },
  })).status()).toBe(200);
  expect((await request.get(`${oauthBaseURL}/api/oauth/servers`, {
    headers: { Authorization: `Bearer ${regrant.access_token}` },
  })).status()).toBe(403);

  await appCard.getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.getByText('OAuth application deleted.')).toBeVisible();
  await expect(dialog.getByText(appName, { exact: true })).toHaveCount(0);
  expect((await request.get(`${oauthBaseURL}/api/oauth/userinfo`, {
    headers: { Authorization: `Bearer ${regrant.access_token}` },
  })).status()).toBe(401);
  expect((await request.post(`${oauthBaseURL}/api/oauth/token`, {
    form: { grant_type: 'refresh_token', client_id: clientId, refresh_token: regrant.refresh_token },
  })).status()).toBe(401);
  await context.close();
});
