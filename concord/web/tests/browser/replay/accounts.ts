import { expect, test } from '@playwright/test';
import { type ChatStore } from './fixtures';

export function registerFacadeSubscribersObserveCoordinatedAccountAndSyncEntityCommitsAtomically() {
  test('facade subscribers observe coordinated account and sync entity commits atomically', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const observed = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.setState({
        activeAccountId: 'account-a', accountGeneration: 1, syncCursor: 'old',
        messages: { old: [{ id: 'old', from: 'old', content: 'old', timestamp: '2026-01-01T00:00:00Z' }] }
      });
      const snapshots: Array<{ account: string | null; generation: number; cursor: string | null; keys: string[] }> = [];
      const unsubscribe = store.subscribe((state) => snapshots.push({
        account: state.activeAccountId, generation: state.accountGeneration,
        cursor: state.syncCursor, keys: Object.keys(state.messages),
      }));
      store.setState({ activeAccountId: 'account-b', accountGeneration: 2, syncCursor: 'new', messages: { fresh: [] } });
      unsubscribe();
      return snapshots;
    });
    expect(observed).toEqual([{ account: 'account-b', generation: 2, cursor: 'new', keys: ['fresh'] }]);
  });
}

export function registerLogoutInvalidatesAHeldAuthenticationCheckAndReportsRevokeFailure() {
  test('logout invalidates a held authentication check and reports revoke failure', async ({ page }) => {
    let releaseMe!: () => void;
    const heldMe = new Promise<void>((resolve) => { releaseMe = resolve; });
    let requestedMe!: () => void;
    const seenMe = new Promise<void>((resolve) => { requestedMe = resolve; });
    await page.route('**/api/auth/status', (route) => route.fulfill({
      status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: ['atproto'] }),
    }));
    await page.route('**/api/me', async (route) => {
      requestedMe();
      await heldMe;
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ id: 'alice', username: 'alice' }) });
    });
    await page.route('**/api/auth/logout', (route) => route.fulfill({ status: 503, body: 'revoke unavailable' }));
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'authStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { authStore: { getState(): { checkAuth(): Promise<void> } } }).authStore;
      (window as typeof window & { heldCheck?: Promise<void> }).heldCheck = store.getState().checkAuth();
    });
    await seenMe;
    await page.evaluate(async () => {
      const store = (window as typeof window & { authStore: { getState(): { logout(): Promise<void> } } }).authStore;
      await store.getState().logout();
    });
    releaseMe();
    await page.evaluate(() => (window as typeof window & { heldCheck: Promise<void> }).heldCheck);
    expect(await page.evaluate(() => {
      const store = (window as typeof window & { authStore: { getState(): { user: unknown; error: string | null } } }).authStore;
      const { user, error } = store.getState();
      return { user, error };
    })).toEqual({ user: null, error: expect.stringContaining('server session could not be revoked') });
  });
}

export function registerAHeldServerFolderLoadCannotOverwriteANewerLocalEditOrAccount() {
  test('a held server folder load cannot overwrite a newer local edit or account', async ({ page }) => {
    let releaseAlice!: () => void;
    const heldAlice = new Promise<void>((resolve) => { releaseAlice = resolve; });
    let getCount = 0;
    await page.route('**/api/auth/status', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: [] }) }));
    await page.route('**/api/me', (route) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ id: 'alice', username: 'alice' }) }));
    await page.route('**/api/server-folders', async (route) => {
      if (route.request().method() === 'PUT') return route.fulfill({ status: 204 });
      getCount += 1;
      if (getCount === 1) {
        await heldAlice;
        return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([{ id: 'old', name: 'Old', server_ids: ['one'] }]) });
      }
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([{ id: 'bob', name: 'Bob', server_ids: ['two'] }]) });
    });
    await page.routeWebSocket(/\/ws/, () => { });
    await page.goto('/app-harness.html');
    await page.waitForFunction(() => 'uiStore' in window);
    await expect.poll(() => getCount).toBe(1);
    await page.evaluate(() => {
      const scope = window as typeof window & {
        uiStore: typeof import('../../../src/stores/uiStore').useUiStore;
        authStore: typeof import('../../../src/stores/authStore').useAuthStore;
      };
      scope.uiStore.getState().addServerFolder('Local', ['local']);
      scope.authStore.setState({ user: { id: 'bob', username: 'bob' } });
    });
    await expect.poll(() => getCount).toBe(2);
    releaseAlice();
    await expect.poll(() => page.evaluate(() => (window as typeof window & { uiStore: typeof import('../../../src/stores/uiStore').useUiStore }).uiStore.getState().serverFolders.map((folder) => folder.name))).toEqual(['Bob']);
  });
}

export function registerHeldIRCTokenCreationCannotRevealAnOldAccountCredentialAfterAccountSwitch() {
  test('held IRC token creation cannot reveal an old account credential after account switch', async ({ page }) => {
    let releaseCreate!: () => void;
    const heldCreate = new Promise<void>((resolve) => { releaseCreate = resolve; });
    let markCreateStarted!: () => void;
    const createStarted = new Promise<void>((resolve) => { markCreateStarted = resolve; });
    await page.route('**/api/tokens', async (route) => {
      if (route.request().method() === 'POST') {
        markCreateStarted();
        await heldCreate;
        await route.fulfill({
          status: 200, contentType: 'application/json', body: JSON.stringify({
            id: 'old-token', token: 'irc-old-account-secret', label: 'old account', created_at: '2026-09-06T00:00:00Z',
          })
        });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: '[]' });
      }
    });
    await page.goto('/settings-harness.html');
    await page.getByPlaceholder('Token label (optional)').fill('old account');
    await page.getByRole('button', { name: 'Generate' }).click();
    await createStarted;
    await page.evaluate(() => {
      const scope = window as typeof window & { authStore: typeof import('../../../src/stores/authStore').useAuthStore; chatStore: ChatStore };
      scope.authStore.setState({ user: { id: 'account-b', username: 'bob' } });
      scope.chatStore.setState({ activeAccountId: 'account-b', protectedGeneration: 2 });
    });
    releaseCreate();
    await expect(page.getByText('irc-old-account-secret')).toHaveCount(0);
    await expect(page.getByPlaceholder('Token label (optional)')).toHaveValue('');
    await expect(page.getByRole('button', { name: 'Generate' })).toBeEnabled();
  });
}

export function registerAuthenticationBootstrapDistinguishesSignedOutFromADependencyFailure() {
  test('authentication bootstrap distinguishes signed out from a dependency failure', async ({ page }) => {
    let meStatus = 503;
    await page.route('**/api/auth/status', (route) => route.fulfill({
      status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: ['atproto'] }),
    }));
    await page.route('**/api/me', (route) => route.fulfill({ status: meStatus, body: meStatus === 401 ? 'unauthorized' : 'dependency unavailable' }));
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'authStore' in window);
    const dependencyFailure = await page.evaluate(async () => {
      const store = (window as typeof window & { authStore: typeof import('../../../src/stores/authStore').useAuthStore }).authStore;
      store.setState({ user: { id: 'alice', username: 'alice' }, error: null });
      await store.getState().checkAuth();
      return { user: store.getState().user, error: store.getState().error };
    });
    expect(dependencyFailure).toEqual({
      user: { id: 'alice', username: 'alice' },
      error: expect.stringContaining('dependency unavailable'),
    });

    meStatus = 401;
    const signedOut = await page.evaluate(async () => {
      const store = (window as typeof window & { authStore: typeof import('../../../src/stores/authStore').useAuthStore }).authStore;
      await store.getState().checkAuth();
      return { user: store.getState().user, error: store.getState().error };
    });
    expect(signedOut).toEqual({ user: null, error: null });
  });
}

export function registerAnAuthenticatedAppKeepsItsWorkspaceAndOffersRetryWhenSignInVerificationIsUnavailable() {
  test('an authenticated app keeps its workspace and offers retry when sign-in verification is unavailable', async ({ page }) => {
    let meStatus = 200;
    await page.route('**/api/auth/status', (route) => route.fulfill({
      status: 200, contentType: 'application/json', body: JSON.stringify({ authenticated: true, providers: ['atproto'] }),
    }));
    await page.route('**/api/me', (route) => route.fulfill({
      status: meStatus,
      contentType: meStatus === 200 ? 'application/json' : 'text/plain',
      body: meStatus === 200 ? JSON.stringify({ id: 'alice', username: 'alice' }) : 'identity service unavailable',
    }));
    await page.routeWebSocket(/\/ws/, () => { });
    await page.goto('/app-harness.html');
    await page.waitForFunction(() => 'authStore' in window);
    await expect.poll(() => page.evaluate(() => Boolean((window as typeof window & { authStore: typeof import('../../../src/stores/authStore').useAuthStore }).authStore.getState().user))).toBe(true);
    meStatus = 503;
    await page.evaluate(() => (window as typeof window & { authStore: typeof import('../../../src/stores/authStore').useAuthStore }).authStore.getState().checkAuth());
    await expect(page.getByRole('alert')).toContainText('identity service unavailable');
    await expect(page.getByRole('button', { name: 'Retry sign-in check' })).toBeVisible();
    expect(await page.evaluate(() => (window as typeof window & { authStore: typeof import('../../../src/stores/authStore').useAuthStore }).authStore.getState().user?.id)).toBe('alice');

    meStatus = 200;
    await page.getByRole('button', { name: 'Retry sign-in check' }).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
  });
}
