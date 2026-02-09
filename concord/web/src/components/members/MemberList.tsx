import { useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import { UserProfilePopup } from './UserProfilePopup';
import type { MemberInfo } from '../../api/types';

const EMPTY_MEMBERS: MemberInfo[] = [];

export function MemberList() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const key = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const members = useChatStore((s) => (key ? s.members[key] ?? EMPTY_MEMBERS : EMPTY_MEMBERS));
  const avatars = useChatStore((s) => s.avatars);
  const [selectedUser, setSelectedUser] = useState<string | null>(null);
  const [popupAnchor, setPopupAnchor] = useState<{ top: number; left: number } | null>(null);

  const handleMemberClick = (nickname: string, e: React.MouseEvent) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setPopupAnchor({ top: rect.top, left: rect.left - 320 });
    setSelectedUser(nickname);
  };

  return (
    <div className="flex h-full w-60 flex-col bg-bg-secondary">
      <div className="px-4 pt-6">
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-muted">
          Members — {members.length}
        </h3>
      </div>

      <div className="flex-1 overflow-y-auto px-2">
        {members.map((member) => {
          const avatarUrl = member.avatar_url || avatars[member.nickname];
          return (
            <button
              key={member.nickname}
              onClick={(e) => handleMemberClick(member.nickname, e)}
              className="flex w-full items-center gap-3 rounded px-2 py-1.5 text-left hover:bg-bg-hover"
            >
              <div className="relative">
                {avatarUrl ? (
                  <img
                    src={avatarUrl}
                    alt={member.nickname}
                    className="h-8 w-8 rounded-full object-cover"
                  />
                ) : (
                  <div className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-accent text-xs font-bold text-white">
                    {member.nickname[0]?.toUpperCase() || '?'}
                  </div>
                )}
                <div className="absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full border-2 border-bg-secondary bg-status-online" />
              </div>
              <span className="truncate text-sm text-text-secondary">{member.nickname}</span>
            </button>
          );
        })}
      </div>

      {selectedUser && popupAnchor && (
        <UserProfilePopup
          nickname={selectedUser}
          position={popupAnchor}
          onClose={() => setSelectedUser(null)}
        />
      )}
    </div>
  );
}
