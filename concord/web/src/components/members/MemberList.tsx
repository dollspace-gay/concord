import { useState, useMemo, useCallback, useEffect, useRef } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import { PresenceIndicator } from '../presence/PresenceIndicator';
import type { MemberInfo, RoleInfo } from '../../api/types';
import { ExternalImage } from '../ExternalImage';

const EMPTY_MEMBERS: MemberInfo[] = [];
const EMPTY_ROLES: RoleInfo[] = [];
const EMPTY_MEMBER_ROLES: Record<string, string[]> = {};

interface ContextMenuState {
  userId: string;
  nickname: string;
  x: number;
  y: number;
  trigger: HTMLElement;
}

export function MemberList() {
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const key = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const members = useChatStore((s) => (key ? s.members[key] ?? EMPTY_MEMBERS : EMPTY_MEMBERS));
  const roles = useChatStore((s) => (activeServer ? s.roles[activeServer] ?? EMPTY_ROLES : EMPTY_ROLES));
  const memberRoles = useChatStore((s) => (activeServer ? s.memberRoles[activeServer] ?? EMPTY_MEMBER_ROLES : EMPTY_MEMBER_ROLES));
  const avatars = useChatStore((s) => s.avatars);
  const presences = useChatStore((s) => s.presences);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const kickMember = useChatStore(s => s.kickMember);
  const banMember = useChatStore(s => s.banMember);
  const timeoutMember = useChatStore(s => s.timeoutMember);

  // Get the top (highest position) role with a color for display
  const roleColorFor = useCallback((member: MemberInfo) => {
    const assigned = member.user_id ? memberRoles[member.user_id] ?? member.role_ids ?? [] : member.role_ids ?? [];
    return [...roles]
      .filter((role) => assigned.includes(role.id) && role.color)
      .sort((left, right) => right.position - left.position)[0]?.color ?? null;
  }, [memberRoles, roles]);

  // Group members under the highest display role available for this server.
  const roleGroups = useMemo(() => {
    const sortedRoles = [...roles].sort((a, b) => b.position - a.position);
    const topRole = sortedRoles.find((r) => !r.is_default && r.color);

    return [{
      roleName: topRole?.name ?? 'Members',
      roleColor: topRole?.color ?? null,
      members,
    }];
  }, [members, roles]);

  const handleContextMenu = useCallback((e: React.MouseEvent, member: MemberInfo) => {
    e.preventDefault();
    if (!member.user_id || !activeServer) return;
    setContextMenu({ userId: member.user_id, nickname: member.nickname, x: e.clientX, y: e.clientY, trigger: e.currentTarget as HTMLElement });
  }, [activeServer]);

  const closeContextMenu = useCallback(() => {
    const trigger = contextMenu?.trigger;
    setContextMenu(null);
    requestAnimationFrame(() => trigger?.focus());
  }, [contextMenu]);

  useEffect(() => {
    if (!contextMenu) return;
    menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
  }, [contextMenu]);

  const handleKick = useCallback(() => {
    if (!contextMenu || !activeServer) return;
    const reason = prompt('Kick reason (optional):') ?? undefined;
    kickMember(activeServer, contextMenu.userId, reason);
    closeContextMenu();
  }, [contextMenu, activeServer, kickMember, closeContextMenu]);

  const handleBan = useCallback(() => {
    if (!contextMenu || !activeServer) return;
    const reason = prompt('Ban reason (optional):') ?? undefined;
    const daysStr = prompt('Delete message history (days, 0-7):', '0');
    const days = daysStr ? parseInt(daysStr, 10) : 0;
    banMember(activeServer, contextMenu.userId, reason, isNaN(days) ? 0 : days);
    closeContextMenu();
  }, [contextMenu, activeServer, banMember, closeContextMenu]);

  const handleTimeout = useCallback(() => {
    if (!contextMenu || !activeServer) return;
    const minutes = prompt('Timeout duration in minutes:', '10');
    if (!minutes) { closeContextMenu(); return; }
    const mins = parseInt(minutes, 10);
    if (isNaN(mins) || mins <= 0) { closeContextMenu(); return; }
    const until = new Date(Date.now() + mins * 60 * 1000).toISOString();
    const reason = prompt('Timeout reason (optional):') ?? undefined;
    timeoutMember(activeServer, contextMenu.userId, until, reason);
    closeContextMenu();
  }, [contextMenu, activeServer, timeoutMember, closeContextMenu]);

  return (
    <div aria-label="Members" className="flex h-full w-60 flex-col bg-bg-secondary">
      {roleGroups.map((group) => (
        <div key={group.roleName}>
          <div className="px-4 pt-6">
            <h3
              className="mb-2 text-xs font-semibold uppercase tracking-wide"
              style={{ color: group.roleColor ?? undefined }}
            >
              {!group.roleColor && <span className="text-text-muted">{group.roleName} — {group.members.length}</span>}
              {group.roleColor && <>{group.roleName} — {group.members.length}</>}
            </h3>
          </div>

          <div className="flex-1 overflow-y-auto px-2">
            {group.members.map((member) => {
              const avatarUrl = member.server_avatar_url || member.avatar_url || avatars[member.nickname];
              const presence = activeServer ? presences[activeServer]?.[member.user_id || ''] : null;
              const statusValue = presence?.status || member.status || 'online';
              const roleColor = roleColorFor(member);
              return (
                <div
                  role="button"
                  tabIndex={0}
                  key={member.nickname}
                  onClick={() => {
                    if (member.user_id) {
                      useUiStore.getState().setShowUserProfile(member.user_id);
                    }
                  }}
                  onKeyDown={(event) => {
                    if ((event.key === 'Enter' || event.key === ' ') && member.user_id) {
                      event.preventDefault();
                      useUiStore.getState().setShowUserProfile(member.user_id);
                    }
                  }}
                  onContextMenu={(e) => handleContextMenu(e, member)}
                  className="group flex w-full items-center gap-3 rounded px-2 py-1.5 text-left hover:bg-bg-hover"
                >
                  <div className="relative">
                    {avatarUrl ? (
                      <ExternalImage
                        src={avatarUrl}
                        alt={member.nickname}
                        label={`${member.nickname} avatar`}
                        privacyScopeKey={`member:${activeServer ?? ''}:${member.user_id ?? member.nickname}:avatar`}
                        className="h-8 w-8 rounded-full object-cover"
                      />
                    ) : (
                      <div className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-accent text-xs font-bold text-white">
                        {member.nickname[0]?.toUpperCase() || '?'}
                      </div>
                    )}
                    <PresenceIndicator
                      status={statusValue}
                      size="md"
                      className="absolute -bottom-0.5 -right-0.5"
                    />
                  </div>
                  <div className="min-w-0 flex-1">
                    <span
                      className="truncate text-sm"
                      style={{ color: roleColor ?? undefined }}
                    >
                      {!roleColor && <span className="text-text-secondary">{member.nickname}</span>}
                      {roleColor && member.nickname}
                    </span>
                    {presence?.custom_status && (
                      <div className="truncate text-xs text-text-muted">
                        {presence.status_emoji && <span className="mr-0.5">{presence.status_emoji}</span>}
                        {presence.custom_status}
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      ))}

      {/* Moderation context menu */}
      {contextMenu && (
        <div
          className="fixed inset-0 z-50"
          onMouseDown={(event) => { if (event.target === event.currentTarget) closeContextMenu(); }}
          onContextMenu={(e) => { e.preventDefault(); closeContextMenu(); }}
        >
          <div
            ref={menuRef}
            role="menu"
            aria-label={`Moderate ${contextMenu.nickname}`}
            className="absolute rounded bg-bg-primary shadow-lg border border-border py-1 min-w-[160px]"
            style={{ top: contextMenu.y, left: contextMenu.x }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(event) => {
              const items = [...(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])];
              const index = items.indexOf(document.activeElement as HTMLButtonElement);
              if (event.key === 'Escape') { event.preventDefault(); closeContextMenu(); return; }
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                event.preventDefault();
                const offset = event.key === 'ArrowDown' ? 1 : -1;
                items[(index + offset + items.length) % items.length]?.focus();
              } else if (event.key === 'Home') { event.preventDefault(); items[0]?.focus(); }
              else if (event.key === 'End') { event.preventDefault(); items.at(-1)?.focus(); }
            }}
          >
            <div className="px-3 py-1.5 text-xs font-semibold text-text-muted border-b border-border mb-1">
              {contextMenu.nickname}
            </div>
            <button
              role="menuitem"
              onClick={handleKick}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-yellow-400 hover:bg-bg-hover"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M13 7l5 5m0 0l-5 5m5-5H6" />
              </svg>
              Kick
            </button>
            <button
              role="menuitem"
              onClick={handleBan}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-red-400 hover:bg-bg-hover"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
              </svg>
              Ban
            </button>
            <button
              role="menuitem"
              onClick={handleTimeout}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-orange-400 hover:bg-bg-hover"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              Timeout
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
