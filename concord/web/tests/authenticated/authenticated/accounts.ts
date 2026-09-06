import { expect, test } from '@playwright/test';
import { existsSync, unlinkSync, writeFileSync } from 'node:fs';
import { captureSocketDiagnostics, openGeneral, sessions } from './fixtures';

export function registerRealAuthenticatedBrowserConnectivitySmokeUsesIsolatedAccounts() {
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
}

export function registerRestartReconnectsMultipleTabsToDurableHistoryAndCredentialRevocationClosesOnlyItsSessions() {
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
}
