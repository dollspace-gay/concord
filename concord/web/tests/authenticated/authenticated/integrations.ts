import { expect, test } from '@playwright/test';
import { existsSync, unlinkSync, writeFileSync } from 'node:fs';
import WebSocket from 'ws';
import { attachRawSocket, botSocket, isErrorFrame, openGeneral, rawFramesFromPage, rawSend, sessions } from './fixtures';

export function registerInstalledBotCompletesPublicAndPrivateComponentJourneysOnRealBrowserSockets() {
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
}

export function registerIntegrationLifecyclePreservesRejectedInputAndRetriesAnInstallAfterReconnect() {
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
}
