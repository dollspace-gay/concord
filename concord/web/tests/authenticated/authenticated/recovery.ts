import { expect, test } from '@playwright/test';
import { attachRawSocket, captureSocketDiagnostics, ircClient, ircTags, openGeneral, rawFramesFromPage, rawSend, registerIrc, sessions } from './fixtures';

export function registerInvalidSearchContinuationIsCorrelatedOverTheRealWebSocket() {
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
}

export function registerADirectMessageCommittedWhileItsRecipientIsOfflineIsDeliveredAfterLogin() {
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
}

export function registerIRCAndBrowserShareCanonicalChannelDeliveryOpaqueHistoryAndReconnectRecovery() {
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
}

export function registerChannelCommitFansOutAndSurvivesARealBrowserReconnect() {
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
}

export function registerOwnerTimesOutAMemberAndReadsTheCommittedAuditRecord() {
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
}
