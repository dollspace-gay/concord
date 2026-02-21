import { useEffect, useMemo, useState, useCallback } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey, Permissions, hasPermission } from '../../api/types';
import type { CategoryInfo, ChannelInfo } from '../../api/types';

const EMPTY_CHANNELS: ChannelInfo[] = [];
const EMPTY_UNREAD: Record<string, number> = {};
const EMPTY_CATEGORIES: CategoryInfo[] = [];

export function ChannelList() {
  const activeServer = useUiStore((s) => s.activeServer);
  const channels = useChatStore((s) => (activeServer ? s.channels[activeServer] ?? EMPTY_CHANNELS : EMPTY_CHANNELS));
  const categories = useChatStore((s) => (activeServer ? s.categories[activeServer] ?? EMPTY_CATEGORIES : EMPTY_CATEGORIES));
  const activeChannel = useUiStore((s) => s.activeChannel);
  const setActiveChannel = useUiStore((s) => s.setActiveChannel);
  const collapsedCategories = useUiStore((s) => s.collapsedCategories);
  const toggleCategory = useUiStore((s) => s.toggleCategory);
  const joinChannel = useChatStore((s) => s.joinChannel);
  const getMembers = useChatStore((s) => s.getMembers);
  const fetchHistory = useChatStore((s) => s.fetchHistory);
  const servers = useChatStore((s) => s.servers);
  const unreadCounts = useChatStore((s) => s.unreadCounts ?? EMPTY_UNREAD);
  const markRead = useChatStore((s) => s.markRead);
  const getUnreadCounts = useChatStore((s) => s.getUnreadCounts);
  const activeMessages = useChatStore((s) => {
    if (!activeServer || !activeChannel) return undefined;
    return s.messages[channelKey(activeServer, activeChannel)];
  });
  const createChannel = useChatStore((s) => s.createChannel);

  const [creatingIn, setCreatingIn] = useState<string | null>(null); // category id or '__uncategorized__'
  const [newChannelName, setNewChannelName] = useState('');
  const [newChannelPrivate, setNewChannelPrivate] = useState(false);

  const activeServerInfo = servers.find((s) => s.id === activeServer);
  const serverName = activeServerInfo?.name ?? 'Concord';
  const canManageChannels = hasPermission(activeServerInfo?.my_permissions ?? 0, Permissions.MANAGE_CHANNELS);

  // Fetch unread counts when server changes
  useEffect(() => {
    if (activeServer) {
      getUnreadCounts(activeServer);
    }
  }, [activeServer, getUnreadCounts]);

  // Auto-mark-read when viewing a channel (clear unread for active channel)
  useEffect(() => {
    if (!activeServer || !activeChannel) return;
    if (activeMessages && activeMessages.length > 0) {
      const lastMsg = activeMessages[activeMessages.length - 1];
      markRead(activeServer, activeChannel, lastMsg.id);
    }
  }, [activeServer, activeChannel, activeMessages, markRead]);

  // Group channels by category, sorted by position
  const grouped = useMemo(() => {
    const sortedCategories = [...categories].sort((a, b) => a.position - b.position);
    const sortedChannels = [...channels].sort((a, b) => a.position - b.position);

    const uncategorized = sortedChannels.filter((ch) => !ch.category_id);
    const groups: { category: CategoryInfo | null; channels: ChannelInfo[] }[] = [];

    // Uncategorized channels go first
    groups.push({ category: null, channels: uncategorized });

    // Then each category with its channels
    for (const cat of sortedCategories) {
      const catChannels = sortedChannels.filter((ch) => ch.category_id === cat.id);
      groups.push({ category: cat, channels: catChannels });
    }

    return groups;
  }, [channels, categories]);

  const handleSelect = (name: string) => {
    if (!activeServer) return;
    setActiveChannel(name);
    joinChannel(activeServer, name);
    getMembers(activeServer, name);
    fetchHistory(activeServer, name);
  };

  const handleCreateChannel = useCallback(() => {
    if (!activeServer || !newChannelName.trim()) return;
    const categoryId = creatingIn === '__uncategorized__' ? undefined : creatingIn ?? undefined;
    createChannel(activeServer, newChannelName.trim(), categoryId, newChannelPrivate || undefined);
    setNewChannelName('');
    setNewChannelPrivate(false);
    setCreatingIn(null);
  }, [activeServer, newChannelName, newChannelPrivate, creatingIn, createChannel]);

  const startCreating = (categoryKey: string) => {
    setCreatingIn(categoryKey);
    setNewChannelName('');
    setNewChannelPrivate(false);
  };

  return (
    <div className="flex h-full flex-col bg-bg-secondary">
      <div className="flex h-12 items-center justify-between border-b border-border-primary px-4">
        <h2 className="font-semibold text-text-primary truncate">{serverName}</h2>
        {activeServer && (hasPermission(activeServerInfo?.my_permissions ?? 0, Permissions.MANAGE_SERVER) || canManageChannels || hasPermission(activeServerInfo?.my_permissions ?? 0, Permissions.MANAGE_ROLES)) && (
          <button
            onClick={() => useUiStore.getState().setShowServerSettings(true)}
            className="rounded p-1 text-text-muted transition-colors hover:text-text-primary"
            title="Server Settings"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
          </button>
        )}
      </div>

      <div className="flex-1 overflow-y-auto px-2 pt-4">
        {grouped.map((group, gi) => {
          const isCollapsed = group.category ? collapsedCategories[group.category.id] : false;
          const categoryKey = group.category?.id ?? '__uncategorized__';

          return (
            <div key={group.category?.id ?? `uncategorized-${gi}`} className="mb-1">
              {/* Category header */}
              {group.category ? (
                <div className="mb-0.5 flex w-full items-center gap-1 px-1 py-1">
                  <button
                    onClick={() => toggleCategory(group.category!.id)}
                    className="flex flex-1 items-center gap-1 text-xs font-semibold uppercase tracking-wide text-text-muted hover:text-text-secondary"
                  >
                    <svg
                      className={`h-3 w-3 transition-transform ${isCollapsed ? '-rotate-90' : ''}`}
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke="currentColor"
                      strokeWidth={2}
                    >
                      <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
                    </svg>
                    <span className="truncate">{group.category.name}</span>
                  </button>
                  {canManageChannels && (
                    <button
                      onClick={() => startCreating(categoryKey)}
                      className="rounded p-0.5 text-text-muted opacity-0 transition-opacity hover:text-text-primary group-hover/cat:opacity-100 [div:hover>&]:opacity-100"
                      title="Create Channel"
                    >
                      <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                      </svg>
                    </button>
                  )}
                </div>
              ) : (
                <div className="mb-0.5 flex items-center justify-between px-2 py-1">
                  <span className="text-xs font-semibold uppercase tracking-wide text-text-muted">
                    Channels
                  </span>
                  {canManageChannels && (
                    <button
                      onClick={() => startCreating(categoryKey)}
                      className="rounded p-0.5 text-text-muted transition-colors hover:text-text-primary"
                      title="Create Channel"
                    >
                      <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                      </svg>
                    </button>
                  )}
                </div>
              )}

              {/* Inline create channel form */}
              {creatingIn === categoryKey && (
                <div className="mb-1 rounded bg-bg-tertiary p-2">
                  <input
                    type="text"
                    value={newChannelName}
                    onChange={(e) => setNewChannelName(e.target.value)}
                    placeholder="channel-name"
                    className="mb-1.5 w-full rounded bg-bg-input px-2 py-1 text-sm text-text-primary placeholder-text-muted outline-none"
                    autoFocus
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') handleCreateChannel();
                      if (e.key === 'Escape') setCreatingIn(null);
                    }}
                  />
                  <div className="flex items-center justify-between">
                    <label className="flex items-center gap-1.5 text-xs text-text-muted">
                      <input
                        type="checkbox"
                        checked={newChannelPrivate}
                        onChange={(e) => setNewChannelPrivate(e.target.checked)}
                        className="rounded"
                      />
                      Private
                    </label>
                    <div className="flex gap-1">
                      <button
                        onClick={() => setCreatingIn(null)}
                        className="rounded px-2 py-0.5 text-xs text-text-muted hover:text-text-primary"
                      >
                        Cancel
                      </button>
                      <button
                        onClick={handleCreateChannel}
                        disabled={!newChannelName.trim()}
                        className="rounded bg-bg-accent px-2 py-0.5 text-xs text-white hover:bg-bg-accent-hover disabled:opacity-50"
                      >
                        Create
                      </button>
                    </div>
                  </div>
                </div>
              )}

              {/* Channel list (hidden when collapsed, unless channel has unread or is active) */}
              {group.channels.map((ch) => {
                const key = channelKey(activeServer!, ch.name);
                const unread = unreadCounts[key] || 0;
                const isActive = activeChannel === ch.name;
                const shouldShow = !isCollapsed || isActive || unread > 0;

                if (!shouldShow) return null;

                return (
                  <button
                    key={ch.name}
                    onClick={() => handleSelect(ch.name)}
                    className={`mb-0.5 flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm transition-colors ${
                      isActive
                        ? 'bg-bg-active text-text-primary'
                        : unread > 0
                          ? 'text-text-primary font-semibold hover:bg-bg-hover'
                          : 'text-text-muted hover:bg-bg-hover hover:text-text-secondary'
                    }`}
                  >
                    {ch.is_private ? (
                      <svg className="h-4 w-4 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                      </svg>
                    ) : (
                      <span className="text-lg leading-none text-text-muted">#</span>
                    )}
                    <span className="min-w-0 flex-1 truncate">{ch.name.replace(/^#/, '')}</span>
                    {unread > 0 && !isActive && (
                      <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-red-500 px-1.5 text-xs font-bold text-white">
                        {unread > 99 ? '99+' : unread}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          );
        })}
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
