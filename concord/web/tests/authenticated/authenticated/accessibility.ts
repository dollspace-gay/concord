import { expect, test } from '@playwright/test';
import { captureSocketDiagnostics, openGeneral, sessions } from './fixtures';

export function registerServerCreationSettingsAndIntegrationsDialogsTrapEscapeAndRestoreFocus() {
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
}
