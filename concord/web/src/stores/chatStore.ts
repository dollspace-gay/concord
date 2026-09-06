import { create } from 'zustand';
import type { ChatState } from './chatStore/types';
import { useComposerStore } from './composerStore';
import { useConnectionStore, type ConnectionState } from './connectionStore';
import { useEntityStore } from './entityStore';
import { usePendingStore } from './pendingStore';

import { createAnnouncementActions } from './chatStore/announcementActions';
import { createAtprotoActions } from './chatStore/atprotoActions';
import { createChannelActions } from './chatStore/channelActions';
import { createCommunityActions } from './chatStore/communityActions';
import { createComposerActions } from './chatStore/composerActions';
import { createConnectionActions } from './chatStore/connectionActions';
import { createConversationActions } from './chatStore/conversationActions';
import { createEmojiActions } from './chatStore/emojiActions';
import { handleChatEvent } from './chatStore/events';
import { createInitialState } from './chatStore/initialState';
import { createIntegrationActions } from './chatStore/integrationActions';
import { createMediaActions } from './chatStore/mediaActions';
import { createMessageActions } from './chatStore/messageActions';
import { createModerationActions } from './chatStore/moderationActions';
import { createNotificationActions } from './chatStore/notificationActions';
import { createOrganizationActions } from './chatStore/organizationActions';
import { createProfileActions } from './chatStore/profileActions';
import { createReadActions } from './chatStore/readActions';
import { createSearchActions } from './chatStore/searchActions';
import { createServerActions } from './chatStore/serverActions';
import { createTemplateActions } from './chatStore/templateActions';
import { createTrackedCommands } from './chatStore/trackedCommands';

const CONNECTION_KEYS = [
  'connected', 'ws', 'nickname', 'activeAccountId', 'accountGeneration',
  'protectedGeneration', 'operationGeneration', 'syncCursor',
  'syncWindowCursors', 'durableMode',
  'ownPresenceStatus',
  'ownRequestedStatus', 'ownCustomStatus', 'ownStatusEmoji',
] as const satisfies ReadonlyArray<keyof ConnectionState>;

let coordinatedDomainUpdate = false;

function updateDomainStores(next: Partial<ChatState>) {
  coordinatedDomainUpdate = true;
  try {
    const connection: Partial<Omit<ConnectionState, 'replace'>> = {};
    for (const key of CONNECTION_KEYS) {
      if (key in next) Object.assign(connection, { [key]: next[key as keyof ChatState] });
    }
    if (Object.keys(connection).length > 0) useConnectionStore.getState().replace(connection);
    if (next.pendingCommands !== undefined) usePendingStore.getState().replace(next.pendingCommands);
    const entities = {
      ...(next.servers !== undefined ? { servers: next.servers } : {}),
      ...(next.channels !== undefined ? { channels: next.channels } : {}),
      ...(next.messages !== undefined ? { messages: next.messages } : {}),
      ...(next.members !== undefined ? { members: next.members } : {}),
      ...(next.directConversations !== undefined ? { directConversations: next.directConversations } : {}),
      ...(next.entityVersions !== undefined ? { entityVersions: next.entityVersions } : {}),
      ...(next.deletedMessageIds !== undefined ? { deletedMessageIds: next.deletedMessageIds } : {}),
    };
    if (Object.keys(entities).length > 0) useEntityStore.getState().replace(entities);
  } finally {
    coordinatedDomainUpdate = false;
  }
}

export const useChatStore = create<ChatState>((rawSet, get) => {
  const set = ((next: ChatState | Partial<ChatState> | ((state: ChatState) => ChatState | Partial<ChatState>), replace?: boolean) => {
    const resolved = typeof next === 'function' ? next(get()) : next;
    updateDomainStores(resolved);
    if (replace === true) rawSet(resolved as ChatState, true);
    else rawSet(resolved);
  }) as typeof rawSet;
  const context = { set, get };
  return {
    ...createInitialState(),
    ...createTrackedCommands(context),
    ...createComposerActions(context),
    ...createConnectionActions(context),
    ...createMessageActions(context),
    ...createReadActions(context),
    ...createChannelActions(context),
    ...createServerActions(context),
    ...createEmojiActions(context),
    ...createOrganizationActions(context),
    ...createProfileActions(context),
    ...createSearchActions(context),
    ...createNotificationActions(context),
    ...createConversationActions(context),
    ...createModerationActions(context),
    ...createCommunityActions(context),
    ...createAnnouncementActions(context),
    ...createTemplateActions(context),
    ...createIntegrationActions(context),
    ...createMediaActions(context),
    ...createAtprotoActions(context),
    handleEvent: (event) => handleChatEvent(context, event),
  };
});

// Keep the legacy ChatState selectors and imperative setState test/integration
// seam as a compatibility facade while composer state has one canonical owner.
const setChatState = useChatStore.setState;

useConnectionStore.subscribe((state) => {
  if (coordinatedDomainUpdate) return;
  setChatState(Object.fromEntries(CONNECTION_KEYS.map((key) => [key, state[key]])) as Partial<ChatState>);
});

usePendingStore.subscribe(({ pendingCommands }) => {
  if (!coordinatedDomainUpdate) setChatState({ pendingCommands });
});

useEntityStore.subscribe(({ replace: _replace, ...entities }) => {
  void _replace;
  if (!coordinatedDomainUpdate) setChatState(entities);
});

useComposerStore.subscribe(({ drafts, compositionFiles, failedCompositions, replyingTo }) => {
  setChatState({ drafts, compositionFiles, failedCompositions, replyingTo });
});

useChatStore.setState = ((next, replace) => {
  const resolved = typeof next === 'function' ? next(useChatStore.getState()) : next;
  if (resolved && typeof resolved === 'object') {
    updateDomainStores(resolved);
    const composer = useComposerStore.getState();
    if ('drafts' in resolved || 'compositionFiles' in resolved
      || 'failedCompositions' in resolved || 'replyingTo' in resolved) {
      composer.replaceState({
        drafts: resolved.drafts ?? composer.drafts,
        compositionFiles: resolved.compositionFiles ?? composer.compositionFiles,
        failedCompositions: resolved.failedCompositions ?? composer.failedCompositions,
        replyingTo: resolved.replyingTo !== undefined ? resolved.replyingTo : composer.replyingTo,
      });
    }
  }
  if (replace === true) {
    setChatState(resolved as ChatState, true);
  } else {
    setChatState(resolved);
  }
}) as typeof useChatStore.setState;
