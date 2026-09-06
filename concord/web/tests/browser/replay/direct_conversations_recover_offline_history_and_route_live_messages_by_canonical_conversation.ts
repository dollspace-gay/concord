import { expect, test } from '@playwright/test';
import { type ChatStore, type UiStore } from './fixtures';

export function registerDirectConversationsRecoverOfflineHistoryAndRouteLiveMessagesByCanonicalConversation() {
  test('direct conversations recover offline history and route live messages by canonical conversation', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: unknown[] = [];
      store.setState({
        nickname: 'carmilla', activeAccountId: 'self', accountGeneration: 2,
        operationGeneration: 'generation', ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
      });
      store.getState().handleEvent({
        type: 'direct_conversation_list', conversations: [{
          id: 'dm-1', peer_id: 'peer-id', peer_username: 'laurelai', peer_avatar_url: null,
          last_message_at: '2026-09-05T01:00:00Z', unread_count: 1,
        }],
      });
      const syncRequest = sent.find((entry) => (entry as { type?: string }).type === 'sync') as { request_id: string };
      store.getState().handleEvent({
        type: 'sync_snapshot', request_id: syncRequest.request_id, snapshot: {
          cursor: 'cursor', operation_generation: 'generation', protocol_version: 2,
          history_before: {}, reactions: [], read_states: [], messages: [{
            attachments: [], content: 'delivered while offline', content_format: 'plain',
            conversation_id: 'dm-1', created_at: '2026-09-05T01:00:00Z', deleted: false,
            edited_at: null, entity_version: 1, mentions: [], message_id: 'offline-1', reply_to: null,
            reply_to_id: null, sender_id: 'peer-id', sender_nick: 'laurelai', sequence: '1',
          }],
        },
      });
      store.getState().handleEvent({
        type: 'message', id: 'live-1', conversation_id: 'dm-1', from: 'laurelai', target: 'carmilla',
        content: 'live', timestamp: '2026-09-05T01:01:00Z', server_id: null,
      });
      const liveProjection = {
        conversation_id: 'dm-1', descriptor: {}, entity_id: 'live-1', entity_type: 'message',
        entity_version: 1, kind: 'message_created', reaction: null, read_state: null,
        message: {
          attachments: [], content: 'live', content_format: 'plain', conversation_id: 'dm-1',
          created_at: '2026-09-05T01:01:00Z', deleted: false, edited_at: null, entity_version: 1,
          mentions: [], message_id: 'live-1', reply_to: null, reply_to_id: null,
          sender_id: 'peer-id', sender_nick: 'laurelai', sequence: '2',
        },
      } as const;
      store.getState().handleEvent({ type: 'durable_event', event: liveProjection });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          ...liveProjection, entity_version: 2, kind: 'message_deleted',
          message: { ...liveProjection.message, entity_version: 2, deleted: true, content: null },
        },
      });
      store.getState().handleEvent({
        type: 'message', id: 'live-1', conversation_id: 'dm-1', from: 'laurelai', target: 'carmilla',
        content: 'stale live copy', timestamp: '2026-09-05T01:01:00Z', server_id: null,
      });
      store.getState().handleEvent({
        type: 'message', id: 'offline-1', conversation_id: 'dm-1', from: 'laurelai', target: 'carmilla',
        content: 'duplicate snapshot copy', timestamp: '2026-09-05T01:00:00Z', server_id: null,
      });
      const accepted = store.getState().sendDirectMessage('dm-1', 'laurelai', 'reply');
      const command = sent.find((entry) => (entry as { type?: string }).type === 'send_direct_message') as { nonce: string };
      store.getState().handleEvent({
        type: 'message_ack', id: 'reply-1', server_id: '', channel: 'laurelai', conversation_id: 'dm-1',
        request_id: command.nonce, client_message_id: command.nonce, sequence: '3',
        persisted_at: '2026-09-05T01:02:00Z', replayed: false, nonce: command.nonce,
      });
      return {
        accepted,
        contents: store.getState().messages['dm:dm-1'].map((message) => message.content),
        ids: store.getState().messages['dm:dm-1'].map((message) => message.id),
        command: sent.find((entry) => (entry as { type?: string }).type === 'send_direct_message'),
      };
    });
    expect(result.accepted).toBe(true);
    expect(result.contents).toEqual(['delivered while offline', '', 'reply']);
    expect(result.ids).toEqual(['offline-1', 'live-1', 'reply-1']);
    expect(result.command).toMatchObject({ type: 'send_direct_message', recipient: 'laurelai', content: 'reply' });
  });
}

export function registerRemovedConversationsPruneProtectedStateAndIgnoreDelayedObsoleteSnapshots() {
  test('removed conversations prune protected state and ignore delayed obsolete snapshots', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const state = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: Array<{ type?: string; request_id?: string }> = [];
      const channel = {
        id: 'removed-channel', conversation_id: 'removed-conversation', server_id: 'server-1',
        name: 'private', topic: '', member_count: 1, category_id: null, position: 0,
        is_private: true, channel_type: 'text', thread_parent_message_id: null,
        archived: false, slowmode_seconds: 0, is_nsfw: false,
      };
      store.setState({
        nickname: 'carmilla', ws: { send: (value: unknown) => { sent.push(structuredClone(value) as typeof sent[number]); return true; } },
      });
      store.getState().handleEvent({ type: 'channel_list', server_id: 'server-1', channels: [channel] });
      const obsolete = sent.find((entry) => entry.type === 'sync')!.request_id!;
      store.setState({
        messages: { 'server-1:private': [{ id: 'secret', from: 'alice', content: 'secret', timestamp: '2026-01-01T00:00:00Z' }] },
        members: { 'server-1:private': [{ nickname: 'alice' }] }, unreadCounts: { 'server-1:private': 1 },
        hasMore: { 'server-1:private': true }, entityVersions: { 'message:secret': 1 },
      });
      store.getState().handleEvent({ type: 'channel_list', server_id: 'server-1', channels: [] });
      store.getState().handleEvent({
        type: 'sync_snapshot', request_id: obsolete, snapshot: {
          cursor: 'obsolete', operation_generation: 'old', protocol_version: 2, history_before: {}, reactions: [], read_states: [],
          messages: [{
            attachments: [], content: 'secret', content_format: 'plain', conversation_id: 'removed-conversation',
            created_at: '2026-01-01T00:00:00Z', deleted: false, edited_at: null, entity_version: 1, mentions: [],
            message_id: 'secret', reply_to: null, reply_to_id: null, sender_id: 'alice', sender_nick: 'alice', sequence: '1'
          }],
        },
      });
      const next = store.getState();
      return { messages: next.messages, members: next.members, unread: next.unreadCounts, versions: next.entityVersions, cursor: next.syncCursor };
    });
    expect(state).toEqual({ messages: {}, members: {}, unread: {}, versions: {}, cursor: null });
  });
}

export function registerARejectedDirectSendRemovesItsOptimisticRowAndRemainsRetryableWithFullContext() {
  test('a rejected direct send removes its optimistic row and remains retryable with full context', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: unknown[] = [];
      store.setState({
        nickname: 'carmilla', activeAccountId: 'self', accountGeneration: 3, operationGeneration: 'generation',
        directConversations: [{ id: 'dm-1', peer_id: 'peer', peer_username: 'laurelai', last_message_at: null, unread_count: 0 }],
        ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } },
        replyingTo: { id: 'prior', from: 'laurelai', content_preview: 'prior' },
      });
      store.getState().sendDirectMessage('dm-1', 'laurelai', 'answer', [{
        id: 'attachment', filename: 'answer.txt', content_type: 'text/plain', file_size: 6, url: '/api/uploads/attachment',
      }]);
      const command = sent[0] as { request_id: string };
      store.getState().handleEvent({
        type: 'command_error', request_id: command.request_id, code: 'FORBIDDEN', message: 'blocked', retryable: false,
      });
      const failed = store.getState().failedCompositions[0];
      return {
        messages: store.getState().messages['dm:dm-1'],
        failed: failed && {
          content: failed.content, attachment: failed.attachments[0]?.id, reply: failed.replyTo?.id,
          conversationId: failed.conversationId, recipient: failed.recipient
        },
      };
    });
    expect(result.messages).toEqual([]);
    expect(result.failed).toEqual({ content: 'answer', attachment: 'attachment', reply: 'prior', conversationId: 'dm-1', recipient: 'laurelai' });
  });
}

export function registerCurrentRulesGatePreservesADisconnectedAcceptanceAndRetriesAfterReconnect() {
  test('current rules gate preserves a disconnected acceptance and retries after reconnect', async ({ page }) => {
    await page.goto('/layout-harness.html');
    await page.waitForFunction(() => 'chatStore' in window && 'uiStore' in window);
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; uiStore: UiStore };
      scope.chatStore.setState({
        connected: false,
        activeAccountId: 'member',
        servers: [{ id: 'server', name: 'Garden', owner_id: 'owner', created_at: '2026-01-01T00:00:00Z' }],
        communitySettings: { server: { server_id: 'server', is_discoverable: false, rules_text: 'Be kind.', rules_accepted: false } },
      });
      scope.uiStore.getState().setActiveServer('server');
    });

    const dialog = page.getByRole('dialog', { name: 'Accept server rules' });
    await expect(dialog).toContainText('Be kind.');
    await dialog.getByRole('button', { name: 'Accept rules' }).click();
    await expect(dialog.getByRole('alert')).toContainText('Not connected');
    await expect(dialog.getByRole('button', { name: 'Accept rules' })).toBeEnabled();

    const requestId = await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
      scope.chatStore.setState({ connected: true, ws: { send: (value: unknown) => { scope.rulesCommand = structuredClone(value) as { request_id: string }; return true; } } });
      return null;
    });
    expect(requestId).toBeNull();
    await dialog.getByRole('button', { name: 'Accept rules' }).click();
    await page.waitForFunction(() => Boolean((window as typeof window & { rulesCommand?: unknown }).rulesCommand));
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
      scope.chatStore.getState().handleEvent({ type: 'lifecycle_command_succeeded', request_id: scope.rulesCommand!.request_id });
    });
    await expect(dialog).toHaveCount(0);

    await page.getByTitle('Community').click();
    const community = page.getByRole('dialog', { name: 'Community' });
    await community.getByRole('button', { name: 'Create Invite' }).click();
    const maxUses = community.getByLabel('Max Uses (0 = unlimited)');
    await maxUses.fill('5');
    await community.getByRole('button', { name: 'Generate Invite' }).click();
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
      scope.chatStore.getState().handleEvent({ type: 'command_error', request_id: scope.rulesCommand!.request_id, code: 'FORBIDDEN', message: 'Invite denied', retryable: false });
    });
    await expect(community.getByRole('alert')).toContainText('Invite denied');
    await expect(maxUses).toHaveValue('5');
    await community.getByRole('button', { name: 'Generate Invite' }).click();
    await page.evaluate(() => {
      const scope = window as typeof window & { chatStore: ChatStore; rulesCommand?: { request_id: string } };
      scope.chatStore.getState().handleEvent({ type: 'lifecycle_command_succeeded', request_id: scope.rulesCommand!.request_id });
    });
    await expect(maxUses).toHaveCount(0);
  });
}

export function registerSnapshotAndReplayRebuildNormalizedConversationState() {
  test('snapshot and replay rebuild normalized conversation state', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);

    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const sent: unknown[] = [];
      store.setState({ nickname: 'carmilla', ws: { send: (value: unknown) => { sent.push(structuredClone(value)); return true; } } });
      store.getState().handleEvent({
        type: 'channel_list',
        server_id: 'server-1',
        channels: [{
          id: 'channel-1', conversation_id: 'channel:6368616E6E656C2D31', server_id: 'server-1',
          name: 'general', topic: '', member_count: 2, category_id: null, position: 0,
          is_private: false, channel_type: 'text', thread_parent_message_id: null,
          archived: false, slowmode_seconds: 0, is_nsfw: false,
        }],
      });
      const baseMessage = {
        attachments: [], content: 'before', content_format: 'plain',
        conversation_id: 'channel:6368616E6E656C2D31', created_at: '2026-01-01T00:00:00Z',
        deleted: false, edited_at: null, entity_version: 1, mentions: [], message_id: 'm1',
        reply_to: null, reply_to_id: null, sender_id: 'user-2', sender_nick: 'alice', sequence: '2',
      };
      const snapshotRequest = sent.find((entry) => (entry as { type?: string }).type === 'sync') as { request_id: string };
      store.getState().handleEvent({
        type: 'sync_snapshot', request_id: snapshotRequest.request_id, snapshot: {
          cursor: 'cursor-1', operation_generation: 'generation-2', protocol_version: 2,
          history_before: { 'channel:6368616E6E656C2D31': 'm0' }, messages: [baseMessage],
          reactions: [{ message_id: 'm1', emoji: 'heart', count: 3, reacted_by_me: true }],
          read_states: [{ conversation_id: 'channel:6368616E6E656C2D31', entity_version: 1, message_id: 'm0', sequence: '1' }],
        },
      });
      store.getState().handleEvent({
        type: 'channel_list', server_id: 'server-1', channels: [...store.getState().channels['server-1'], {
          id: 'channel-2', conversation_id: 'channel:6368616E6E656C2D32', server_id: 'server-1',
          name: 'other', topic: '', member_count: 2, category_id: null, position: 1,
          is_private: false, channel_type: 'text', thread_parent_message_id: null,
          archived: false, slowmode_seconds: 0, is_nsfw: false,
        }],
      });
      const replayRequest = [...sent].reverse().find((entry) => (entry as { type?: string }).type === 'sync') as { request_id: string };
      store.getState().handleEvent({
        type: 'replay_batch', request_id: replayRequest.request_id, batch: {
          cursor: 'cursor-2', operation_generation: 'generation-2', protocol_version: 2, has_more: false,
          events: [{
            conversation_id: 'channel:6368616E6E656C2D31', descriptor: {}, entity_id: 'm1',
            entity_type: 'message', entity_version: 2, kind: 'message_updated',
            message: { ...baseMessage, content: 'after', edited_at: '2026-01-01T00:01:00Z', entity_version: 2 },
            reaction: null, read_state: null,
          }],
        },
      });
      const state = store.getState();
      return {
        cursor: state.syncCursor,
        resumeCursor: (sent.findLast((entry) => (entry as { type?: string }).type === 'sync') as { cursor?: string }).cursor,
        generation: state.operationGeneration,
        content: state.messages['server-1:general'][0].content,
        reaction: state.messages['server-1:general'][0].reactions[0],
        unread: state.unreadCounts['server-1:general'],
        hasMore: state.hasMore['server-1:general'],
      };
    });

    expect(result).toEqual({
      cursor: 'cursor-2', resumeCursor: undefined, generation: 'generation-2', content: 'after',
      reaction: { emoji: 'heart', count: 3, user_ids: ['__self__'] }, unread: 1, hasMore: true,
    });
  });
}

export function registerResyncClearsProtectedProjectionsBeforeRebuilding() {
  test('resync clears protected projections before rebuilding', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const state = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      store.setState({
        servers: [{ id: 'server-1', name: 'Server', member_count: 1 }],
        channels: { 'server-1': [{ id: 'channel-1' }] }, messages: { stale: [{ id: 'm1' }] },
        members: { stale: [{ nickname: 'alice' }] }, roles: { 'server-1': [{ id: 'r1' }] },
        categories: { 'server-1': [{ id: 'c1' }] }, pinnedMessages: { stale: [{ id: 'm1' }] },
        threads: { stale: [{ id: 't1' }] }, bookmarks: [{ id: 'b1' }], unreadCounts: { stale: 2 },
        operationGeneration: 'old', syncCursor: 'old',
        drafts: { 'server-1:general': 'unsent draft' },
      });
      store.getState().handleEvent({ type: 'resync_required', reason: 'cursor expired' });
      const next = store.getState();
      return {
        servers: next.servers, channels: next.channels, messages: next.messages, members: next.members,
        roles: next.roles, categories: next.categories, pins: next.pinnedMessages,
        threads: next.threads, bookmarks: next.bookmarks, unread: next.unreadCounts,
        generation: next.operationGeneration, cursor: next.syncCursor,
        drafts: next.drafts,
      };
    });
    expect(state).toEqual({
      servers: [], channels: {}, messages: {}, members: {}, roles: {}, categories: {}, pins: {},
      threads: {}, bookmarks: [], unread: {}, generation: null, cursor: null,
      drafts: { 'server-1:general': 'unsent draft' },
    });
  });
}

export function registerSingleAndBulkDeletesRedactRepliesAndEvictLoadedSearchPinAndBookmarkCopies() {
  test('single and bulk deletes redact replies and evict loaded search pin and bookmark copies', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const message = (id: string, replyId?: string) => ({
        id, from: 'alice', sender_id: 'alice', sequence: id === 'first' ? '1' : '2', content: id,
        timestamp: '2026-09-06T00:00:00Z', attachments: [],
        reply_to: replyId ? { id: replyId, from: 'alice', content_preview: `preview-${replyId}` } : null,
      });
      const pin = (id: string) => ({
        id: `pin-${id}`, message_id: id, channel_id: 'channel', pinned_by: 'alice',
        pinned_at: '2026-09-06T00:00:00Z', from: 'alice', content: id, timestamp: '2026-09-06T00:00:00Z'
      });
      const bookmark = (id: string) => ({
        id: `bookmark-${id}`, message_id: id, channel_id: 'channel', from: 'alice',
        content: id, timestamp: '2026-09-06T00:00:00Z', created_at: '2026-09-06T00:00:00Z'
      });
      const search = (id: string) => ({
        id, from: 'alice', content: id, timestamp: '2026-09-06T00:00:00Z',
        channel_id: 'channel', channel_name: '#general'
      });
      store.setState({
        durableMode: false,
        messages: { 'server:#general': [message('first'), message('reply-first', 'first'), message('second'), message('reply-second', 'second')] },
        pinnedMessages: { 'server:#general': [pin('first'), pin('second')] },
        bookmarks: [bookmark('first'), bookmark('second')],
        searchResults: [search('first'), search('second')],
      });
      store.getState().handleEvent({ type: 'message_delete', id: 'first', server_id: 'server', channel: '#general' });
      store.getState().handleEvent({ type: 'bulk_message_delete', message_ids: ['second'], server_id: 'server', channel: '#general' });
      const state = store.getState();
      return {
        messages: state.messages['server:#general'].map((entry) => ({ id: entry.id, preview: entry.reply_to?.content_preview })),
        pins: state.pinnedMessages['server:#general'], bookmarks: state.bookmarks, search: state.searchResults,
      };
    });
    expect(result).toEqual({
      messages: [{ id: 'reply-first', preview: '' }, { id: 'reply-second', preview: '' }],
      pins: [], bookmarks: [], search: [],
    });
  });
}

export function registerADelayedReadProjectionPreservesMessagesWithANewerCanonicalSequence() {
  test('a delayed read projection preserves messages with a newer canonical sequence', async ({ page }) => {
    await page.goto('/test-harness.html');
    const result = await page.evaluate(async () => {
      const { useChatStore: store } = await import('/src/stores/chatStore.ts');
      store.setState({
        activeAccountId: 'self-id',
        nickname: 'same-nickname',
        directConversations: [{
          id: 'opaque-direct', peer_id: 'peer-id', peer_username: 'same-nickname',
          last_message_at: null, unread_count: 0,
        }],
        messages: {}, unreadCounts: {}, entityVersions: {}, readSequences: {},
      });
      const event = (messageId: string, sequence: string) => ({
        conversation_id: 'opaque-direct', descriptor: {}, entity_id: messageId,
        entity_type: 'message', entity_version: 1, kind: 'message_created',
        message: {
          attachments: [], content: messageId, content_format: 'plain',
          conversation_id: 'opaque-direct', created_at: `2026-01-01T00:00:0${sequence}Z`,
          deleted: false, edited_at: null, entity_version: 1, mentions: [], message_id: messageId,
          reply_to: null, reply_to_id: null, sender_id: 'peer-id', sender_nick: 'same-nickname', sequence,
        },
        reaction: null, read_state: null,
      });
      store.getState().handleEvent({ type: 'durable_event', event: event('m6', '6') });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          conversation_id: 'opaque-direct', descriptor: {}, entity_id: 'read:opaque-direct',
          entity_type: 'read_state', entity_version: 1, kind: 'read_advanced', message: null, reaction: null,
          read_state: { conversation_id: 'opaque-direct', entity_version: 1, message_id: 'm5', sequence: '5' },
        }
});
      const afterDelayedRead = store.getState().unreadCounts['dm:opaque-direct'];
      store.getState().handleEvent({
        type: 'durable_event', event: {
          conversation_id: 'opaque-direct', descriptor: {}, entity_id: 'read:opaque-direct',
          entity_type: 'read_state', entity_version: 2, kind: 'read_advanced', message: null, reaction: null,
          read_state: { conversation_id: 'opaque-direct', entity_version: 2, message_id: 'm6', sequence: '6' },
        }
});
      return {
        afterDelayedRead,
        afterCurrentRead: store.getState().unreadCounts['dm:opaque-direct'],
        senderId: store.getState().messages['dm:opaque-direct'][0].sender_id,
        sequence: store.getState().messages['dm:opaque-direct'][0].sequence,
      };
    });
    expect(result).toEqual({
      afterDelayedRead: 1, afterCurrentRead: 0, senderId: 'peer-id', sequence: '6',
    });
  });
}

export function registerDurableThreadStateUpdatesKnownProjectionsAndIgnoresUnknownDescriptors() {
  test('durable thread state updates known projections and ignores unknown descriptors', async ({ page }) => {
    await page.goto('/test-harness.html');
    await page.waitForFunction(() => 'chatStore' in window);
    const result = await page.evaluate(() => {
      const store = (window as typeof window & { chatStore: ChatStore }).chatStore;
      const conversation = 'channel:thread-1';
      const thread = {
        id: 'thread-1', name: 'topic', channel_type: 'public_thread', parent_message_id: null,
        archived: false, auto_archive_minutes: 1440, message_count: 0,
        created_at: '2026-09-01T00:00:00Z',
      };
      store.setState({
        channels: {
          server: [{
            id: 'thread-1', conversation_id: conversation, server_id: 'server', name: 'topic',
            topic: '', member_count: 1, category_id: null, position: 0, is_private: false,
            channel_type: 'public_thread', thread_parent_message_id: null, archived: false,
            slowmode_seconds: 0, is_nsfw: false,
          }]
        },
        threads: { 'server:parent': [thread] },
      });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          conversation_id: conversation, descriptor: { archived: true, reason: 'manual' },
          entity_id: 'thread-1', entity_type: 'thread_state', entity_version: 2,
          kind: 'thread_state_changed', message: null, reaction: null, read_state: null,
        }
      });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          conversation_id: conversation, descriptor: { thread_id: 'thread-1', tag_ids: ['tag-new'] },
          entity_id: 'thread-1', entity_type: 'thread_tags', entity_version: 5,
          kind: 'thread_tags_updated', message: null, reaction: null, read_state: null,
        }
      });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          conversation_id: conversation, descriptor: { thread_id: 'thread-1', tag_ids: ['tag-stale'] },
          entity_id: 'thread-1', entity_type: 'thread_tags', entity_version: 4,
          kind: 'thread_tags_updated', message: null, reaction: null, read_state: null,
        }
      });
      store.getState().handleEvent({
        type: 'durable_event', event: {
          conversation_id: conversation, descriptor: { future: true }, entity_id: 'future-state',
          entity_type: 'future_type', entity_version: 9, kind: 'future_kind',
          message: null, reaction: null, read_state: null,
        }
      });
      const state = store.getState();
      return {
        channelArchived: state.channels.server[0].archived,
        threadArchived: state.threads['server:parent'][0].archived,
        threadVersion: state.entityVersions['thread_state:thread-1'],
        tagVersion: state.entityVersions['thread_tags:thread-1'],
        tagIds: state.threads['server:parent'][0].tag_ids,
        futureVersion: state.entityVersions['future_type:future-state'],
      };
    });
    expect(result).toEqual({
      channelArchived: true,
      threadArchived: true,
      threadVersion: 2,
      tagVersion: 5,
      tagIds: ['tag-new'],
      futureVersion: undefined,
    });
  });
}
