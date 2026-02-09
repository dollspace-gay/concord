import { useUiStore } from '../../stores/uiStore';
import { ServerList } from '../servers/ServerList';
import { ChannelList } from '../channels/ChannelList';
import { ChannelHeader } from '../chat/ChannelHeader';
import { MessageInput } from '../chat/MessageInput';
import { MessageList } from '../chat/MessageList';
import { MemberList } from '../members/MemberList';
import { SettingsPage } from '../auth/SettingsPage';

export function AppLayout() {
  const showMemberList = useUiStore((s) => s.showMemberList);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const showSettings = useUiStore((s) => s.showSettings);

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
        <ChannelHeader />
        <MessageList />
        {activeChannel && <MessageInput />}
      </div>

      {/* Member list sidebar */}
      {showMemberList && activeChannel && <MemberList />}

      {/* Settings modal */}
      {showSettings && <SettingsPage />}
    </div>
  );
}
