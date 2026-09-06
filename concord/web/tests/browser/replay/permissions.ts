import { expect, test } from '@playwright/test';
import { type ChatStore } from './fixtures';

export function registerHiddenTabsHonorNotificationSettingsAndClaimOneDesktopAlertPerMessage() {
  test('hidden tabs honor notification settings and claim one desktop alert per message', async ({ page }) => {
    await page.addInitScript(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
      (window as typeof window & { notifications: Array<{ title: string; body?: string }> }).notifications = [];
      class FakeNotification {
        static permission = 'granted';
        constructor(title: string, options?: NotificationOptions) {
          (window as typeof window & { notifications: Array<{ title: string; body?: string }> }).notifications.push({
            title, body: options?.body,
          });
        }
      }
      Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore; notifications: unknown[] }).chatStore;
      const channel = {
        id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
        member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
        thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
      };
      store.setState({
        activeAccountId: 'alice', nickname: 'alice', channels: { server: [channel] },
        notificationSettings: {
          server: [{
            id: 'setting', server_id: 'server', channel_id: null, level: 'mentions',
            suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null,
          }]
        },
      });
      const projection = {
        type: 'durable_event' as const,
        event: {
          kind: 'message_created', conversation_id: 'conversation', entity_type: 'message', entity_id: 'message',
          entity_version: 1, reaction: null, read_state: null, descriptor: {},
          message: {
            attachments: [], content: 'hello @alice', content_format: 'markdown', conversation_id: 'conversation',
            created_at: '2026-09-06T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
            mentions: [{ kind: 'user' as const, target_id: 'alice', start_byte: 6, end_byte: 12 }],
            message_id: 'message', reply_to: null, reply_to_id: null, sender_id: 'bob', sender_nick: 'bob', sequence: '1',
          },
        },
      };
      store.getState().handleEvent(projection);
      // Simulate a second tab that has not projected the message yet. The shared
      // localStorage claim must still prevent a duplicate desktop notification.
      store.setState({ messages: {}, entityVersions: {} });
      store.getState().handleEvent(projection);
      store.setState({
        memberRoles: { server: { alice: ['role-1'] } },
        messages: {}, entityVersions: {},
      });
      store.getState().handleEvent({
        ...projection, event: {
          ...projection.event, entity_id: 'role-message',
          message: {
            ...projection.event.message, message_id: 'role-message', sequence: '2', content: 'structured role',
            mentions: [{ kind: 'role', target_id: 'role-1', start_byte: 0, end_byte: 0 }]
          },
        }
      });
      store.getState().handleEvent({
        ...projection, event: {
          ...projection.event, entity_id: 'text-only-message',
          message: { ...projection.event.message, message_id: 'text-only-message', sequence: '3', content: 'hello @alice', mentions: [] },
        }
      });
      store.setState({
        notificationSettings: {
          server: [
            { id: 'global', server_id: null, channel_id: null, level: 'all', suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null },
            { id: 'server-default', server_id: 'server', channel_id: null, level: 'default', suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null },
            { id: 'channel-default', server_id: 'server', channel_id: 'channel', level: 'default', suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null },
          ]
        },
      });
      store.getState().handleEvent({
        ...projection, event: {
          ...projection.event, entity_id: 'inherited-message',
          message: { ...projection.event.message, message_id: 'inherited-message', sequence: '4', content: 'inherited all', mentions: [] },
        }
      });
    });
    await expect.poll(() => page.evaluate(() =>
      (window as typeof window & { notifications: unknown[] }).notifications,
    )).toEqual([
      { title: 'bob', body: 'hello @alice' },
      { title: 'bob', body: 'structured role' },
      { title: 'bob', body: 'inherited all' },
    ]);
  });
}

export function registerReconnectedRoleMembershipEnablesRoleAlertsUntilALiveRemoval() {
  test('reconnected role membership enables role alerts until a live removal', async ({ page }) => {
    await page.addInitScript(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
      (window as typeof window & { notifications: string[] }).notifications = [];
      class FakeNotification {
        static permission = 'granted';
        constructor(_title: string, options?: NotificationOptions) {
          (window as typeof window & { notifications: string[] }).notifications.push(options?.body ?? '');
        }
      }
      Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
    });
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const handleEvent = store.getState().handleEvent as (event: unknown) => void;
      store.setState({
        activeAccountId: 'alice', nickname: 'alice', memberRoles: {},
        channels: {
          server: [{
            id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
            member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
            thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
          }]
        },
        notificationSettings: {
          server: [{
            id: 'setting', server_id: 'server', channel_id: null, level: 'mentions',
            suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null,
          }]
        },
      });
      // The authoritative NAMES snapshot is replayed after reconnect and must
      // restore the current account's role assignments before messages arrive.
      handleEvent({
        type: 'names', server_id: 'server', channel: '#general', members: [{
          nickname: 'alice', user_id: 'alice', avatar_url: null, server_avatar_url: null,
          status: 'online', custom_status: null, status_emoji: null, role_ids: ['role-on-call'],
        }],
      });
      const projection = {
        type: 'durable_event',
        event: {
          kind: 'message_created', conversation_id: 'conversation', entity_type: 'message',
          entity_id: 'role-after-reconnect', entity_version: 1, reaction: null, read_state: null, descriptor: {},
          message: {
            attachments: [], content: 'assigned role alert', content_format: 'plain', conversation_id: 'conversation',
            created_at: '2026-09-06T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
            mentions: [{ kind: 'role', target_id: 'role-on-call', start_byte: 0, end_byte: 0 }],
            message_id: 'role-after-reconnect', reply_to: null, reply_to_id: null,
            sender_id: 'bob', sender_nick: 'bob', sequence: '1',
          },
        },
      };
      handleEvent(projection);
      handleEvent({ type: 'member_role_update', server_id: 'server', version: 1, user_id: 'alice', role_ids: [] });
      handleEvent({
        ...projection, event: {
          ...projection.event, entity_id: 'role-after-removal',
          message: { ...projection.event.message, message_id: 'role-after-removal', sequence: '2', content: 'removed role alert' },
        }
      });
    });
    await expect.poll(() => page.evaluate(() =>
      (window as typeof window & { notifications: string[] }).notifications,
    )).toEqual(['assigned role alert']);
  });
}

export function registerLateRoleSnapshotsCannotResurrectRemovedAssignmentsOrColors() {
  test('late role snapshots cannot resurrect removed assignments or colors', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const handleEvent = store.getState().handleEvent as (event: unknown) => void;
      const role = {
        id: 'colored', server_id: 'server', name: 'Colored', color: '#123456', icon_url: null,
        position: 1, permissions: 0, is_default: false,
      };
      // Capture the committed version 1 snapshot, then deliver version 2 first.
      const members = Array.from({ length: 300 }, (_, index) => ({
        user_id: `user-${index}`, role_ids: ['colored'],
      }));
      handleEvent({ type: 'role_list', server_id: 'server', version: 1, roles: [role], member_roles: members });
      const heldOld = {
        type: 'role_list', server_id: 'server', version: 1, roles: [role],
        member_roles: members,
      };
      handleEvent({ type: 'role_list', server_id: 'server', version: 2, roles: [], member_roles: undefined });
      handleEvent({ type: 'member_role_update', server_id: 'server', version: 2, user_id: 'alice', role_ids: [] });
      handleEvent(heldOld);
      handleEvent({ type: 'member_role_update', server_id: 'server', version: 1, user_id: 'alice', role_ids: ['colored'] });
      return {
        roles: store.getState().roles.server,
        assignments: store.getState().memberRoles.server?.alice,
        remainingAssignments: Object.values(store.getState().memberRoles.server ?? {}).flat().length,
        memberCount: Object.keys(store.getState().memberRoles.server ?? {}).length,
        version: store.getState().roleProjectionVersions.server,
      };
    });
    expect(result).toEqual({
      roles: [], assignments: [], remainingAssignments: 0, memberCount: 301, version: 2,
    });
  });
}

export function registerConcurrentTabsAtomicallyClaimOneDesktopNotification() {
  test('concurrent tabs atomically claim one desktop notification', async ({ context }) => {
    await context.addInitScript(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
      (window as typeof window & { notifications: string[] }).notifications = [];
      class FakeNotification {
        static permission = 'granted';
        constructor(title: string) {
          (window as typeof window & { notifications: string[] }).notifications.push(title);
        }
      }
      Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
    });
    const pages = await Promise.all([context.newPage(), context.newPage()]);
    await Promise.all(pages.map(async (page) => {
      await page.goto('/test-harness.html');
      await page.waitForFunction(() => 'chatStore' in window);
    }));
    await Promise.all(pages.map((page) => page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.setState({
        activeAccountId: 'atomic-alice', nickname: 'alice',
        channels: {
          server: [{
            id: 'channel', conversation_id: 'conversation', server_id: 'server', name: '#general', topic: '',
            member_count: 2, category_id: null, position: 0, is_private: false, channel_type: 'text',
            thread_parent_message_id: null, archived: false, slowmode_seconds: 0, is_nsfw: false,
          }]
        },
        notificationSettings: {
          server: [{
            id: 'setting', server_id: 'server', channel_id: null, level: 'all',
            suppress_everyone: false, suppress_roles: false, muted: false, mute_until: null,
          }]
        },
      });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          kind: 'message_created', conversation_id: 'conversation', entity_type: 'message', entity_id: 'atomic-message',
          entity_version: 1, reaction: null, read_state: null, descriptor: {},
          message: {
            attachments: [], content: 'one alert', content_format: 'plain', conversation_id: 'conversation',
            created_at: '2026-09-06T00:00:00Z', deleted: false, edited_at: null, entity_version: 1,
            mentions: [], message_id: 'atomic-message', reply_to: null, reply_to_id: null,
            sender_id: 'bob', sender_nick: 'bob', sequence: '1',
          },
        },
      });
    })));
    await expect.poll(async () => (await Promise.all(pages.map((page) => page.evaluate(() =>
      (window as typeof window & { notifications: string[] }).notifications.length,
    )))).reduce((sum, count) => sum + count, 0)).toBe(1);
  });
}
