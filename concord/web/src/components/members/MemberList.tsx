import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';

export function MemberList() {
  const activeChannel = useUiStore((s) => s.activeChannel);
  const members = useChatStore((s) => (activeChannel ? s.members[activeChannel] || [] : []));

  return (
    <div className="flex h-full w-60 flex-col bg-bg-secondary">
      <div className="px-4 pt-6">
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-muted">
          Members — {members.length}
        </h3>
      </div>

      <div className="flex-1 overflow-y-auto px-2">
        {members.map((nick) => (
          <div
            key={nick}
            className="flex items-center gap-3 rounded px-2 py-1.5 hover:bg-bg-hover"
          >
            <div className="relative">
              <div className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-accent text-xs font-bold text-white">
                {nick[0]?.toUpperCase() || '?'}
              </div>
              <div className="absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full border-2 border-bg-secondary bg-status-online" />
            </div>
            <span className="truncate text-sm text-text-secondary">{nick}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
