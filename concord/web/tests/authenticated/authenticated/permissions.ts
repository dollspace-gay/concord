import { expect, test } from '@playwright/test';
import { existsSync, unlinkSync, writeFileSync } from 'node:fs';
import { captureSocketDiagnostics, openGeneral, sessions } from './fixtures';

export function registerOwnerManagesMultipleServersCategoriesAndImmediatePrivateVisibility() {
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
}

export function registerOwnerManagesColoredRolePermissionsAndMemberAssignment() {
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
}

export function registerPrivateThreadRemainsScopedAndArchiveStateSurvivesRestart() {
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
}
