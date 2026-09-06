import { expect, test } from '@playwright/test';
import { type ChatStore, type UiStore } from './fixtures';

export function registerResyncClearsEveryProtectedCacheThroughTheSharedReset() {
  test('resync clears every protected cache through the shared reset', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const marker = { private: true };
      (store.setState as unknown as (state: Record<string, unknown>) => void)({
        servers: [marker], channels: { private: [marker] }, messages: { private: [marker] },
        members: { private: [marker] }, hasMore: { private: true }, avatars: { private: 'x' },
        typingUsers: { private: ['x'] }, replyingTo: marker, unreadCounts: { private: 1 },
        customEmoji: { private: marker }, roles: { private: [marker] }, categories: { private: [marker] },
        presences: { private: marker }, userProfiles: { private: marker }, searchResults: [marker],
        searchQuery: 'private', searchTotalCount: 1, pinnedMessages: { private: [marker] },
        threads: { private: [marker] }, forumTags: { private: [marker] }, bookmarks: [marker], notificationSettings: { private: [marker] },
        auditLog: { private: [marker] }, bans: { private: [marker] }, automodRules: { private: [marker] },
        invites: { private: [marker] }, serverEvents: { private: [marker] },
        communitySettings: { private: marker }, discoverableServers: [marker], templates: { private: [marker] },
        webhooks: { private: [marker] }, slashCommands: { private: [marker] }, botTokens: [marker],
        oauth2Apps: [marker], blueskyIdentities: { private: marker }, atprotoSyncEnabled: true,
        stickers: { private: [marker] }, allUserEmoji: [marker], serverAvatars: { private: marker },
        maxMessageLength: 1, maxFileSizeMb: 1,
      });
      store.getState().handleEvent({ type: 'resync_required', reason: 'authority changed' });
      const state = store.getState() as unknown as Record<string, unknown>;
      const emptyCollections = [
        'servers', 'channels', 'messages', 'members', 'hasMore', 'avatars', 'typingUsers',
        'unreadCounts', 'customEmoji', 'roles', 'categories', 'presences', 'userProfiles',
        'pinnedMessages', 'threads', 'forumTags', 'bookmarks', 'notificationSettings', 'auditLog', 'bans', 'automodRules',
        'invites', 'serverEvents', 'communitySettings', 'discoverableServers', 'templates',
        'webhooks', 'slashCommands', 'botTokens', 'oauth2Apps', 'blueskyIdentities', 'stickers',
        'allUserEmoji', 'serverAvatars',
      ];
      const leaks = emptyCollections.filter((key) => {
        const value = state[key];
        return Array.isArray(value) ? value.length !== 0 : Object.keys(value as object).length !== 0;
      });
      if (state.replyingTo !== null) leaks.push('replyingTo');
      if (state.searchResults !== null || state.searchQuery !== '' || state.searchTotalCount !== 0) leaks.push('search');
      if (state.atprotoSyncEnabled !== false) leaks.push('atprotoSyncEnabled');
      return { leaks, maxMessageLength: state.maxMessageLength, maxFileSizeMb: state.maxFileSizeMb };
    });
    expect(result).toEqual({ leaks: [], maxMessageLength: 4000, maxFileSizeMb: 100 });
  });
}

export function registerAProtectedHTTPResponseHeldAcrossResyncCannotRepopulatePrivateState() {
  test('a protected HTTP response held across resync cannot repopulate private state', async ({ page }) => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    let requested!: () => void;
    const seen = new Promise<void>((resolve) => { requested = resolve; });
    await page.route('**/api/servers/private/emoji', async (route) => {
      requested();
      await held;
      await route.fulfill({
        status: 200, contentType: 'application/json', body: JSON.stringify([
          { id: 'private-emoji', server_id: 'private', name: 'secret', image_url: '/secret.png' },
        ])
      });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.setState({ activeAccountId: 'alice' });
      (window as typeof window & { heldLoad?: Promise<void> }).heldLoad = store.getState().loadServerEmoji('private');
    });
    await seen;
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.getState().handleEvent({ type: 'resync_required', reason: 'permission changed' });
    });
    release();
    await page.evaluate(() => (window as typeof window & { heldLoad: Promise<void> }).heldLoad);
    expect(await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      return store.getState().customEmoji;
    })).toEqual({});
  });
}

export function registerChannelReadStateAdvancesOnlyAfterTheConversationTabBecomesVisible() {
  test('channel read state advances only after the conversation tab becomes visible', async ({ page }) => {
    await page.addInitScript(() => {
      (window as typeof window & { testVisibility: DocumentVisibilityState }).testVisibility = 'hidden';
      Object.defineProperty(document, 'visibilityState', {
        configurable: true,
        get: () => (window as typeof window & { testVisibility: DocumentVisibilityState }).testVisibility,
      });
    });
    await page.goto('/layout-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sent: Array<{ type: string }> };
      stores.sent = [];
      stores.chatStore.setState({
        activeAccountId: 'alice', nickname: 'alice', operationGeneration: 'generation',
        ws: { send: (command: { type: string }) => { stores.sent.push(structuredClone(command)); return true; } },
        channels: {
          server: [{
            id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
            member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
            thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
          }]
        },
        messages: {
          'server:#general': [{
            id: 'message', from: 'bob', sender_id: 'bob', sequence: '7', content: 'unread',
            timestamp: '2026-09-06T00:00:00Z', attachments: [],
          }]
        },
      });
      stores.uiStore.getState().setActiveServer('server');
      stores.uiStore.getState().setActiveChannel('#general');
    });
    await page.waitForTimeout(50);
    expect(await page.evaluate(() => (window as typeof window & { sent: Array<{ type: string }> }).sent
      .filter((command) => command.type === 'mark_read').length)).toBe(0);
    await page.evaluate(() => {
      (window as typeof window & { testVisibility: DocumentVisibilityState }).testVisibility = 'visible';
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await expect.poll(() => page.evaluate(() => (window as typeof window & { sent: Array<{ type: string }> }).sent
      .filter((command) => command.type === 'mark_read').length)).toBe(1);
  });
}

export function registerMoreThanOneHundredVisibleConversationsSynchronizeInBoundedCompleteWindows() {
  test('more than one hundred visible conversations synchronize in bounded complete windows', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: Array<{ type: string; request_id?: string; subscriptions?: string[] }> = [];
      store.setState({
        ws: {
          send: (command: { type: string; request_id?: string; subscriptions?: string[] }) => {
            sent.push(structuredClone(command)); return true;
          }
        }
      });
      const channels = Array.from({ length: 205 }, (_, index) => ({
        id: `channel-${index}`, conversation_id: `conversation-${index}`, server_id: 'server',
        name: `channel-${index}`, topic: '', member_count: 1, category_id: null, position: index,
        is_private: false, channel_type: 'text', thread_parent_message_id: null, archived: false,
        slowmode_seconds: 0, is_nsfw: false,
      }));
      store.getState().handleEvent({ type: 'channel_list', server_id: 'server', channels });
      const syncs = sent.filter((command) => command.type === 'sync');
      const respond = (sync: typeof syncs[number], omitFirst = false) => {
        const subscriptions = (sync.subscriptions ?? []).slice(omitFirst ? 1 : 0);
        store.getState().handleEvent({
          type: 'sync_snapshot', request_id: sync.request_id!, snapshot: {
            cursor: `cursor-${sync.request_id}`, operation_generation: 'generation', protocol_version: 2,
            history_before: {}, reactions: [], read_states: [], messages: subscriptions.map((conversation, index) => ({
              attachments: [], content: conversation, content_format: 'plain', conversation_id: conversation,
              created_at: '2026-09-01T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
              mentions: [], message_id: `message-${conversation}`, reply_to: null, reply_to_id: null,
              sender_id: 'peer', sender_nick: 'peer', sequence: String(index + 1),
            })),
          }
        });
      };
      syncs.forEach((sync) => respond(sync));
      const firstCount = Object.values(store.getState().messages).flat().length;
      store.getState().handleEvent({ type: 'channel_list', server_id: 'server', channels: channels.slice(1) });
      const repeated = sent.filter((command) => command.type === 'sync').slice(-3);
      repeated.forEach((sync) => respond(sync));
      return {
        sizes: syncs.map((command) => command.subscriptions?.length),
        subscriptions: new Set(syncs.flatMap((command) => command.subscriptions ?? [])).size,
        firstCount,
        repeatedCount: Object.values(store.getState().messages).flat().length,
      };
    });
    expect(result).toEqual({
      sizes: [100, 100, 5], subscriptions: 205, firstCount: 205, repeatedCount: 204,
    });
  });
}

export function registerANonretryableServerRejectionRemovesTheOptimisticRowWithoutOverwritingNewerComposition() {
  test('a nonretryable server rejection removes the optimistic row without overwriting newer composition', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: Array<{ request_id: string }> = [];
      store.setState({
        nickname: 'carmilla', activeAccountId: 'account-a', operationGeneration: 'generation-2',
        ws: { send: (command: { request_id: string }) => { sent.push(command); return true; } },
      });
      store.getState().sendMessage('server-1', 'general', 'rejected composition');
      store.getState().setDraft('server-1:general', 'newer composition');
      store.getState().handleEvent({
        type: 'command_error', request_id: sent[0].request_id, code: 'INVALID_INPUT',
        message: 'rejected', retryable: false,
      });
      return {
        draft: store.getState().drafts['server-1:general'],
        messages: store.getState().messages['server-1:general'] ?? [],
        pending: Object.keys(store.getState().pendingCommands),
      };
    });
    expect(result).toEqual({ draft: 'newer composition', messages: [], pending: [] });
  });
}

export function registerSearchValidatesInputAndUsesCorrelatedContinuationPagesWithDeletionTombstones() {
  test('search validates input and uses correlated continuation pages with deletion tombstones', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const stores = window as typeof window & { chatStore: ChatStore; uiStore: UiStore; sentCommands: unknown[] };
      stores.sentCommands = [];
      stores.chatStore.setState({ ws: { send: (command: unknown) => { stores.sentCommands.push(structuredClone(command)); return true; } } });
      stores.uiStore.getState().setActiveServer('server-1');
      stores.uiStore.getState().setShowSearch(true);
    });
    const search = page.getByPlaceholder('Search messages...');
    await search.fill('before:yesterday');
    await search.press('Enter');
    await expect(page.getByRole('alert')).toContainText('Invalid before: date');
    expect(await page.evaluate(() => (window as typeof window & { sentCommands: unknown[] }).sentCommands)).toEqual([]);

    await search.fill('from:alice release notes');
    await search.press('Enter');
    await expect.poll(() => page.evaluate(() => {
      const commands = (window as typeof window & { sentCommands: Array<{ type?: string; limit?: number; offset?: number }> }).sentCommands;
      return commands.at(-1);
    })).toMatchObject({ type: 'search_messages', limit: 25, offset: 0 });
    await page.evaluate(() => {
      const scope = window as typeof window & {
        chatStore: ChatStore;
        sentCommands: Array<{ request_id?: string }>;
      };
      const requestId = scope.sentCommands.at(-1)!.request_id!;
      scope.chatStore.getState().handleEvent({
        type: 'search_results', request_id: requestId, server_id: 'server-1',
        query: 'from:alice release notes', results: [{
          id: 'deleted-result', from: 'alice', content: 'old', timestamp: '2026-01-01T00:00:00Z',
          channel_id: 'channel-1', channel_name: 'general',
        }], total_count: 60, offset: 0, next_continuation: 'page-2', restarted: false,
      });
    });
    await page.getByRole('button', { name: 'Next' }).click();
    await expect.poll(() => page.evaluate(() => {
      const commands = (window as typeof window & { sentCommands: Array<{ type?: string; offset?: number; continuation?: string }> }).sentCommands;
      return commands.at(-1);
    })).toMatchObject({ offset: 25, continuation: 'page-2' });
    const state = await page.evaluate(() => {
      const scope = window as typeof window & {
        chatStore: ChatStore;
        sentCommands: Array<{ request_id?: string }>;
      };
      const staleRequest = scope.sentCommands.at(-2)!.request_id!;
      const currentRequest = scope.sentCommands.at(-1)!.request_id!;
      scope.chatStore.getState().handleEvent({
        type: 'message_delete', server_id: 'server-1', channel: '#general', id: 'deleted-result',
      });
      const stalePage = {
        type: 'search_results' as const, server_id: 'server-1', query: 'from:alice release notes',
        results: [{ id: 'deleted-result', from: 'alice', content: 'stale', timestamp: '2026-01-01T00:00:00Z', channel_id: 'channel-1', channel_name: 'general' }],
        total_count: 60, offset: 0, next_continuation: 'stale-token', restarted: false,
      };
      scope.chatStore.getState().handleEvent({ ...stalePage, request_id: staleRequest });
      scope.chatStore.getState().handleEvent({ ...stalePage, request_id: currentRequest, offset: 25, next_continuation: null });
      return {
        results: scope.chatStore.getState().searchResults,
        deleted: scope.chatStore.getState().deletedMessageIds,
        offset: scope.chatStore.getState().searchOffset,
      };
    });
    expect(state).toEqual({ results: [], deleted: { 'deleted-result': true }, offset: 25 });
  });
}
