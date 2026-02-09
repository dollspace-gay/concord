import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import type { ChannelInfo } from '../../api/types';

const EMPTY_CHANNELS: ChannelInfo[] = [];

export function ChannelHeader() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const channels = useChatStore((s) => (activeServer ? s.channels[activeServer] ?? EMPTY_CHANNELS : EMPTY_CHANNELS));
  const toggleMemberList = useUiStore((s) => s.toggleMemberList);
  const showMemberList = useUiStore((s) => s.showMemberList);

  const channel = channels.find((c) => c.name === activeChannel);

  if (!activeChannel) {
    return <div className="flex h-12 items-center border-b border-border-primary bg-bg-tertiary px-4" />;
  }

  return (
    <div className="flex h-12 items-center justify-between border-b border-border-primary bg-bg-tertiary px-4">
      <div className="flex min-w-0 items-center gap-2">
        <span className="text-lg text-text-muted">#</span>
        <span className="font-semibold text-text-primary">
          {activeChannel.replace(/^#/, '')}
        </span>
        {channel?.topic && (
          <>
            <span className="mx-2 text-text-muted">|</span>
            <span className="truncate text-sm text-text-muted">{channel.topic}</span>
          </>
        )}
      </div>

      <button
        onClick={toggleMemberList}
        className={`rounded p-1.5 transition-colors ${
          showMemberList ? 'text-text-primary' : 'text-text-muted hover:text-text-secondary'
        }`}
        title="Toggle member list"
      >
        <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
          <path d="M16 11c1.66 0 2.99-1.34 2.99-3S17.66 5 16 5c-1.66 0-3 1.34-3 3s1.34 3 3 3zm-8 0c1.66 0 2.99-1.34 2.99-3S9.66 5 8 5C6.34 5 5 6.34 5 8s1.34 3 3 3zm0 2c-2.33 0-7 1.17-7 3.5V19h14v-2.5c0-2.33-4.67-3.5-7-3.5zm8 0c-.29 0-.62.02-.97.05 1.16.84 1.97 1.97 1.97 3.45V19h6v-2.5c0-2.33-4.67-3.5-7-3.5z" />
        </svg>
      </button>
    </div>
  );
}
