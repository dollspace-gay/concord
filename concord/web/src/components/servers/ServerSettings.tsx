import { useEffect, useState } from 'react';
import { hasPermission, Permissions } from '../../api/types';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { Dialog } from '../Dialog';
import { CategoriesTab } from './settings/CategoriesTab';
import { ChannelsTab } from './settings/ChannelsTab';
import { EmojiTab } from './settings/EmojiTab';
import { OverviewTab } from './settings/OverviewTab';
import { RolesTab } from './settings/RolesTab';
import { StickersTab } from './settings/StickersTab';
import { EMPTY_CATEGORIES, EMPTY_CHANNELS, EMPTY_EMOJI, EMPTY_FORUM_TAGS, EMPTY_MEMBER_ROLES, EMPTY_ROLES, EMPTY_STICKERS } from './settings/defaults';

type Tab = 'overview' | 'channels' | 'roles' | 'categories' | 'emoji' | 'stickers';

export function ServerSettings() {
  const activeServer = useUiStore((s) => s.activeServer);
  const setShowServerSettings = useUiStore((s) => s.setShowServerSettings);
  const roles = useChatStore((s) => (activeServer ? s.roles[activeServer] ?? EMPTY_ROLES : EMPTY_ROLES));
  const categories = useChatStore((s) => (activeServer ? s.categories[activeServer] ?? EMPTY_CATEGORIES : EMPTY_CATEGORIES));
  const channels = useChatStore((s) => (activeServer ? s.channels[activeServer] ?? EMPTY_CHANNELS : EMPTY_CHANNELS));
  const customEmoji = useChatStore((s) => (activeServer ? s.customEmoji[activeServer] ?? EMPTY_EMOJI : EMPTY_EMOJI));
  const servers = useChatStore((s) => s.servers);
  const createRole = useChatStore((s) => s.createRole);
  const updateRole = useChatStore((s) => s.updateRole);
  const deleteRole = useChatStore((s) => s.deleteRole);
  const assignRole = useChatStore((s) => s.assignRole);
  const removeRole = useChatStore((s) => s.removeRole);
  const memberRoles = useChatStore((s) => (activeServer ? s.memberRoles[activeServer] ?? EMPTY_MEMBER_ROLES : EMPTY_MEMBER_ROLES));
  const channelPermissionOverrides = useChatStore((s) => s.channelPermissionOverrides);
  const listChannelPermissionOverrides = useChatStore((s) => s.listChannelPermissionOverrides);
  const setChannelPermissionOverride = useChatStore((s) => s.setChannelPermissionOverride);
  const deleteChannelPermissionOverride = useChatStore((s) => s.deleteChannelPermissionOverride);
  const createCategory = useChatStore((s) => s.createCategory);
  const updateCategory = useChatStore((s) => s.updateCategory);
  const deleteCategory = useChatStore((s) => s.deleteCategory);
  const updateServer = useChatStore((s) => s.updateServer);
  const deleteServer = useChatStore((s) => s.deleteServer);
  const createChannel = useChatStore((s) => s.createChannel);
  const deleteChannel = useChatStore((s) => s.deleteChannel);
  const loadServerEmoji = useChatStore((s) => s.loadServerEmoji);
  const createEmoji = useChatStore((s) => s.createEmoji);
  const deleteEmoji = useChatStore((s) => s.deleteEmoji);
  const stickers = useChatStore((s) => (activeServer ? s.stickers[activeServer] ?? EMPTY_STICKERS : EMPTY_STICKERS));
  const loadServerStickers = useChatStore((s) => s.loadServerStickers);
  const createSticker = useChatStore((s) => s.createSticker);
  const deleteSticker = useChatStore((s) => s.deleteSticker);
  const setVanityCode = useChatStore((s) => s.setVanityCode);
  const forumTags = useChatStore((s) => s.forumTags ?? EMPTY_FORUM_TAGS);
  const createForumTag = useChatStore((s) => s.createForumTag);
  const updateForumTag = useChatStore((s) => s.updateForumTag);
  const deleteForumTag = useChatStore((s) => s.deleteForumTag);
  const listForumTags = useChatStore((s) => s.listForumTags);

  const [tab, setTab] = useState<Tab>('overview');

  const server = servers.find((s) => s.id === activeServer);
  const serverName = server?.name ?? 'Server';

  // Load emoji/stickers when switching to their tabs
  useEffect(() => {
    if (tab === 'emoji' && activeServer) {
      loadServerEmoji(activeServer);
    }
    if (tab === 'stickers' && activeServer) {
      loadServerStickers(activeServer);
    }
  }, [tab, activeServer, loadServerEmoji, loadServerStickers]);

  if (!activeServer) return null;

  const permissions = server?.my_permissions ?? 0;
  const canManageServer = hasPermission(permissions, Permissions.MANAGE_SERVER);
  const canManageChannels = hasPermission(permissions, Permissions.MANAGE_CHANNELS);
  const canManageRoles = hasPermission(permissions, Permissions.MANAGE_ROLES);
  const tabs: { key: Tab; label: string }[] = [
    ...(canManageServer ? [{ key: 'overview' as const, label: 'Overview' }] : []),
    ...(canManageChannels ? [
      { key: 'channels' as const, label: 'Channels' },
      { key: 'categories' as const, label: 'Categories' },
    ] : []),
    ...(canManageRoles ? [{ key: 'roles' as const, label: 'Roles' }] : []),
    ...(canManageServer ? [
      { key: 'emoji' as const, label: 'Emoji' },
      { key: 'stickers' as const, label: 'Stickers' },
    ] : []),
  ];
  const visibleTab = tabs.some((candidate) => candidate.key === tab) ? tab : tabs[0]?.key;

  return (
    <Dialog label={`${serverName} Settings`} onClose={() => setShowServerSettings(false)} panelClassName="max-h-[80vh] w-full max-w-2xl overflow-y-auto rounded-lg bg-bg-secondary p-6">
      <div className="mb-6 flex items-center justify-between">
        <h2 className="text-xl font-bold text-text-primary">{serverName} Settings</h2>
        <button
          onClick={() => setShowServerSettings(false)}
          title="Close settings"
          aria-label="Close settings"
          className="rounded p-1 text-text-muted transition-colors hover:text-text-primary"
        >
          <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Tab bar */}
      <div className="mb-6 flex gap-1 rounded-lg bg-bg-primary p-1">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={`flex-1 rounded-md px-3 py-2 text-sm font-medium transition-colors ${visibleTab === t.key ? 'bg-bg-accent text-white' : 'text-text-muted hover:text-text-primary'
              }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {visibleTab === 'overview' && (
        <OverviewTab serverId={activeServer} server={server!} updateServer={updateServer} setVanityCode={setVanityCode} deleteServer={deleteServer} close={() => setShowServerSettings(false)} />
      )}
      {visibleTab === 'channels' && (
        <ChannelsTab
          serverId={activeServer}
          channels={channels}
          categories={categories}
          createChannel={createChannel}
          deleteChannel={deleteChannel}
          forumTags={forumTags}
          createForumTag={createForumTag}
          updateForumTag={updateForumTag}
          deleteForumTag={deleteForumTag}
          listForumTags={listForumTags}
          roles={roles}
          channelPermissionOverrides={channelPermissionOverrides}
          listChannelPermissionOverrides={listChannelPermissionOverrides}
          setChannelPermissionOverride={setChannelPermissionOverride}
          deleteChannelPermissionOverride={deleteChannelPermissionOverride}
        />
      )}
      {visibleTab === 'roles' && (
        <RolesTab
          serverId={activeServer}
          roles={roles}
          createRole={createRole}
          updateRole={updateRole}
          deleteRole={deleteRole}
          assignRole={assignRole}
          removeRole={removeRole}
          memberRoles={memberRoles}
        />
      )}
      {visibleTab === 'categories' && (
        <CategoriesTab
          serverId={activeServer}
          categories={categories}
          createCategory={createCategory}
          updateCategory={updateCategory}
          deleteCategory={deleteCategory}
        />
      )}
      {visibleTab === 'emoji' && (
        <EmojiTab
          serverId={activeServer}
          emoji={customEmoji}
          createEmoji={createEmoji}
          deleteEmoji={deleteEmoji}
        />
      )}
      {visibleTab === 'stickers' && (
        <StickersTab
          serverId={activeServer}
          stickers={stickers}
          createSticker={createSticker}
          deleteSticker={deleteSticker}
        />
      )}
    </Dialog>
  );
}
