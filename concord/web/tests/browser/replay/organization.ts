import { expect, test } from '@playwright/test';
import { type ChatStore, type UiStore } from './fixtures';

export function registerServerAndProfileImagesRemainOptInBeforeThirdPartyRequests() {
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
}

export function registerServerFolderSavesSerializeLatestStructureAndRetainFailedEditsForRetry() {
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
      const store = (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore;
      store.getState().hydrateServerFolders('alice');
      store.getState().addServerFolder('First', ['one']);
      store.getState().addServerFolder('Second', ['two']);
    });
    await expect.poll(() => requests).toBe(1);
    releaseFirst();
    await expect.poll(() => requests).toBe(2);
    await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore.getState().folderSyncStatus)).toBe('idle');
    expect(payloads[1]).toMatchObject([{ name: 'First' }, { name: 'Second' }]);

    failNext = true;
    await page.evaluate(() => {
      const store = (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore;
      store.getState().addServerFolder('Unsaved', ['three']);
    });
    await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore.getState().folderSyncStatus)).toBe('error');
    expect(await page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore.getState().serverFolders.map((folder) => folder.name))).toContain('Unsaved');
    await page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore.getState().retryServerFolderSync());
    await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore.getState().folderSyncStatus)).toBe('idle');
  });
}
