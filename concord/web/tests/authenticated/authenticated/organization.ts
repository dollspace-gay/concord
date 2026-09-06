import { expect, test } from '@playwright/test';
import { captureSocketDiagnostics, openGeneral, sessions } from './fixtures';

export function registerOwnerCreatesAForumAndManagesItsTagsThroughTheRealServer() {
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
}

export function registerOwnerSnapshotsAndInstantiatesAServerTemplateThroughTheRealServer() {
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
}

export function registerAMemberEditsProfilePresenceAndServerIdentityThroughSettings() {
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
}

export function registerOwnerManagesLocalEmojiAndStickersThroughTheBrowserWithPolicyAndRevocation() {
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
}
