import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { ServerList } from '../servers/ServerList';
import { ChannelList } from '../channels/ChannelList';
import { ChannelHeader } from '../chat/ChannelHeader';
import { MessageInput } from '../chat/MessageInput';
import { MessageList } from '../chat/MessageList';
import { MemberList } from '../members/MemberList';
import { SettingsPage } from '../auth/SettingsPage';
import { ServerSettings } from '../servers/ServerSettings';
import { QuickSwitcher } from '../navigation/QuickSwitcher';
import { UserProfileModal } from '../profiles/UserProfileModal';
import { SearchPanel } from '../search/SearchPanel';
import { PinnedMessagesPanel } from '../pins/PinnedMessagesPanel';
import { ThreadPanel } from '../threads/ThreadPanel';
import { BookmarksPanel } from '../bookmarks/BookmarksPanel';
import { ModerationPanel } from '../moderation/ModerationPanel';
import { CommunityPanel } from '../community/CommunityPanel';
import { IntegrationsPanel } from '../integrations/IntegrationsPanel';

export function AppLayout() {
  const showMemberList = useUiStore((s) => s.showMemberList);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const showSettings = useUiStore((s) => s.showSettings);
  const showServerSettings = useUiStore((s) => s.showServerSettings);
  const showSearch = useUiStore((s) => s.showSearch);
  const setShowSearch = useUiStore((s) => s.setShowSearch);
  const showPinnedMessages = useUiStore((s) => s.showPinnedMessages);
  const setShowPinnedMessages = useUiStore((s) => s.setShowPinnedMessages);
  const showBookmarks = useUiStore((s) => s.showBookmarks);
  const setShowBookmarks = useUiStore((s) => s.setShowBookmarks);
  const showThreadPanel = useUiStore((s) => s.showThreadPanel);
  const showModerationPanel = useUiStore((s) => s.showModerationPanel);
  const setShowModerationPanel = useUiStore((s) => s.setShowModerationPanel);
  const showCommunityPanel = useUiStore((s) => s.showCommunityPanel);
  const setShowCommunityPanel = useUiStore((s) => s.setShowCommunityPanel);
  const showIntegrationsPanel = useUiStore((s) => s.showIntegrationsPanel);
  const setShowIntegrationsPanel = useUiStore((s) => s.setShowIntegrationsPanel);
  const activeServer = useUiStore((s) => s.activeServer);
  const errorToast = useChatStore((s) => s.errorToast);

  return (
    <div className="flex h-full">
      {/* Server icon strip */}
      <ServerList />

      {/* Channel sidebar */}
      <div className="w-60 shrink-0">
        <ChannelList />
      </div>

      {/* Main chat area */}
      <div className="flex min-w-0 flex-1 flex-col bg-bg-tertiary">
        <div className="flex items-center">
          <div className="flex-1">
            <ChannelHeader />
          </div>
          <div className="flex items-center gap-1 border-b border-border-primary bg-bg-tertiary pr-2">
            {/* Pin icon button */}
            <button
              onClick={() => setShowPinnedMessages(!showPinnedMessages)}
              className={`rounded p-1.5 transition-colors ${
                showPinnedMessages ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Pinned messages"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25v13.5m-7.5-13.5v13.5" />
              </svg>
            </button>
            {/* Bookmark icon button */}
            <button
              onClick={() => setShowBookmarks(!showBookmarks)}
              className={`rounded p-1.5 transition-colors ${
                showBookmarks ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Bookmarks"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 5a2 2 0 012-2h10a2 2 0 012 2v16l-7-3.5L5 21V5z" />
              </svg>
            </button>
            {/* Community icon button */}
            <button
              onClick={() => setShowCommunityPanel(!showCommunityPanel)}
              className={`rounded p-1.5 transition-colors ${
                showCommunityPanel ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Community"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 017.843 4.582M12 3a8.997 8.997 0 00-7.843 4.582m15.686 0A11.953 11.953 0 0112 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0121 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0112 16.5a17.92 17.92 0 01-8.716-2.247m0 0A8.966 8.966 0 013 12c0-1.777.514-3.434 1.401-4.83" />
              </svg>
            </button>
            {/* Integrations icon button */}
            <button
              onClick={() => setShowIntegrationsPanel(!showIntegrationsPanel)}
              className={`rounded p-1.5 transition-colors ${
                showIntegrationsPanel ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Integrations"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M14.25 9.75L16.5 12l-2.25 2.25m-4.5 0L7.5 12l2.25-2.25M6 20.25h12A2.25 2.25 0 0020.25 18V6A2.25 2.25 0 0018 3.75H6A2.25 2.25 0 003.75 6v12A2.25 2.25 0 006 20.25z" />
              </svg>
            </button>
            {/* Moderation icon button */}
            <button
              onClick={() => setShowModerationPanel(!showModerationPanel)}
              className={`rounded p-1.5 transition-colors ${
                showModerationPanel ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Moderation"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z" />
              </svg>
            </button>
            {/* Search icon button */}
            <button
              onClick={() => setShowSearch(!showSearch)}
              className={`rounded p-1.5 transition-colors ${
                showSearch ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
              title="Search messages"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
              </svg>
            </button>
          </div>
        </div>
        <MessageList />
        {activeChannel && <MessageInput />}
      </div>

      {/* Member list sidebar */}
      {showMemberList && activeChannel && <MemberList />}

      {/* Pinned messages panel */}
      {showPinnedMessages && <PinnedMessagesPanel />}

      {/* Thread panel */}
      {showThreadPanel && <ThreadPanel />}

      {/* Bookmarks panel */}
      {showBookmarks && <BookmarksPanel />}

      {/* Search panel sidebar */}
      {showSearch && <SearchPanel />}

      {/* Settings modal */}
      {showSettings && <SettingsPage />}

      {/* Server settings modal */}
      {showServerSettings && <ServerSettings />}

      {/* Moderation panel modal */}
      {showModerationPanel && activeServer && (
        <ModerationPanel serverId={activeServer} onClose={() => setShowModerationPanel(false)} />
      )}

      {/* Community panel modal */}
      {showCommunityPanel && activeServer && (
        <CommunityPanel serverId={activeServer} onClose={() => setShowCommunityPanel(false)} />
      )}

      {/* Integrations panel modal */}
      {showIntegrationsPanel && activeServer && (
        <IntegrationsPanel serverId={activeServer} onClose={() => setShowIntegrationsPanel(false)} />
      )}

      {/* Quick switcher modal (Ctrl+K) */}
      <QuickSwitcher />

      {/* User profile modal */}
      <UserProfileModal />

      {/* Error toast */}
      {errorToast && (
        <div className="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-red-600 px-4 py-2 text-sm text-white shadow-lg">
          {errorToast}
        </div>
      )}
    </div>
  );
}
