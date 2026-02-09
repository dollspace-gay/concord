import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';

export function ChannelList() {
  const channels = useChatStore((s) => s.channels);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const setActiveChannel = useUiStore((s) => s.setActiveChannel);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const getMembers = useChatStore((s) => s.getMembers);
  const fetchHistory = useChatStore((s) => s.fetchHistory);

  const handleSelect = (name: string) => {
    setActiveChannel(name);
    joinChannel(name);
    getMembers(name);
    fetchHistory(name);
  };

  return (
    <div className="flex h-full flex-col bg-bg-secondary">
      <div className="flex h-12 items-center border-b border-border-primary px-4">
        <h2 className="font-semibold text-text-primary">Concord</h2>
      </div>

      <div className="flex-1 overflow-y-auto px-2 pt-4">
        <div className="mb-2 flex items-center justify-between px-2">
          <span className="text-xs font-semibold uppercase tracking-wide text-text-muted">
            Channels
          </span>
        </div>

        {channels.map((ch) => (
          <button
            key={ch.name}
            onClick={() => handleSelect(ch.name)}
            className={`mb-0.5 flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm transition-colors ${
              activeChannel === ch.name
                ? 'bg-bg-active text-text-primary'
                : 'text-text-muted hover:bg-bg-hover hover:text-text-secondary'
            }`}
          >
            <span className="text-lg leading-none text-text-muted">#</span>
            <span className="truncate">{ch.name.replace(/^#/, '')}</span>
          </button>
        ))}
      </div>

      <div className="border-t border-border-primary px-2 py-2">
        <UserBar />
      </div>
    </div>
  );
}

function UserBar() {
  const setShowSettings = useUiStore((s) => s.setShowSettings);

  return (
    <button
      onClick={() => setShowSettings(true)}
      className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-sm text-text-secondary transition-colors hover:bg-bg-hover"
    >
      <div className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-accent text-xs font-bold text-white">
        U
      </div>
      <span className="truncate">Settings</span>
    </button>
  );
}
