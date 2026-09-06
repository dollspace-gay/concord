import { expect, test } from '@playwright/test';
import { type ChatStore, type UiStore } from './fixtures';

export function registerSlashCommandAutocompleteIsKeyboardAccessibleAndSubmitsTypedArguments() {
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
        slashCommands: {
          server: [{
            id: 'hello-command', bot_user_id: 'bot', server_id: 'server', name: 'hello',
            description: 'Greet a member', created_at: '2026-09-06T12:00:00Z',
            options: [{ name: 'member', description: 'Member name', option_type: 'string', required: true }],
          }]
        },
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
}

export function registerFormattedMessagesExposeSafeLinksAndKeyboardOperableSpoilers() {
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
}

export function registerImageViewerTrapsDismissalAndRestoresFocusToItsOpener() {
  test('image viewer traps dismissal and restores focus to its opener', async ({ page }) => {
    await page.route('**/api/uploads/image', (route) => route.fulfill({
      status: 200, contentType: 'image/gif', body: Buffer.from('R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==', 'base64'),
    }));
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.uiStore.setState({ activeServer: 'server', activeChannel: 'general', activeDirectConversation: null });
      scope.chatStore.setState({
        messages: {
          'server:general': [{
            id: 'image-message', from: 'alice', content: '', timestamp: '2026-09-06T00:00:00Z',
            attachments: [{ id: 'image', filename: 'proof.gif', content_type: 'image/gif', file_size: 1, url: '/api/uploads/image' }],
          }]
        }
      });
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
}

export function registerNestedDialogsSkipHiddenControlsAndEscapeClosesOnlyTheTopDialog() {
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
}

export function registerWorkspaceSettingsDialogsOpenAndCloseByKeyboardWithFocusRestoration() {
  test('workspace settings dialogs open and close by keyboard with focus restoration', async ({ page }) => {
    await page.goto('/layout-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.chatStore.setState({
        servers: [{ id: 'server', name: 'Garden', owner_id: 'alice', created_at: '2026-01-01T00:00:00Z', my_permissions: 8 }],
        channels: {
          server: [{
            id: 'general', conversation_id: 'conversation', server_id: 'server', name: 'general', topic: '',
            member_count: 1, category_id: null, position: 0, is_private: false, channel_type: 'text',
            thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false
          }]
        }, activeAccountId: 'alice'
      });
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
}

export function registerMobileChatUsesTheFullViewportAndProvidesAConversationBackAction() {
  test('mobile chat uses the full viewport and provides a conversation back action', async ({ page }) => {
    await page.setViewportSize({ width: 360, height: 667 });
    await page.goto('/layout-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      stores.chatStore.setState({
        nickname: 'carmilla', activeAccountId: 'self', operationGeneration: 'generation',
        channels: {
          'server-1': [{
            id: 'channel', conversation_id: 'conversation', server_id: 'server-1', name: 'general',
            topic: '', member_count: 1, position: 0, is_private: false, channel_type: 'text', archived: false,
            slowmode_seconds: 0, is_nsfw: false
          }]
        },
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
}

export function registerWorkspaceRemainsUsableWithoutHorizontalOverflowAtTabletAndDesktopWidths() {
  test('workspace remains usable without horizontal overflow at tablet and desktop widths', async ({ page }) => {
    for (const width of [768, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/layout-harness.html');
      await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
      await page.evaluate(() => {
        const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
        scope.chatStore.setState({
          nickname: 'alice', activeAccountId: 'alice', channels: {
            server: [{
              id: 'general', conversation_id: 'conversation', server_id: 'server', name: 'general', topic: '',
              member_count: 1, category_id: null, position: 0, is_private: false, channel_type: 'text',
              thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
            }]
          }
        });
        scope.uiStore.getState().setActiveServer('server');
        scope.uiStore.getState().setActiveChannel('general');
      });
      await expect(page.getByPlaceholder('Message general')).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    }
  });
}

export function registerQuickSwitcherIsKeyboardNavigableAndRestoresFocusAfterSelection() {
  test('quick switcher is keyboard navigable and restores focus after selection', async ({ page }) => {
    await page.goto('/layout-harness.html');
    await page.evaluate(() => {
      const typed = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      typed.chatStore.setState({
        servers: [{ id: 'server-1', name: 'Night School', member_count: 1, role: 'owner', my_permissions: 0 }],
        channels: {
          'server-1': [{
            id: 'channel-1', conversation_id: 'conversation-1', server_id: 'server-1', name: '#general',
            topic: '', member_count: 1, category_id: null, position: 0, is_private: false,
            channel_type: 'text', thread_parent_message_id: null, archived: false,
            slowmode_seconds: 0, is_nsfw: false,
          }]
        },
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
}
