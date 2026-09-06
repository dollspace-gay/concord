import { useState, useEffect, useRef } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import type { RoleInfo, CategoryInfo, ChannelInfo, StickerInfo, ForumTagInfo } from '../../api/types';
import type { ChannelPermissionOverrideInfo } from '../../api/generated/contract';
import { channelKey, hasPermission, Permissions } from '../../api/types';
import { getAtprotoChannelPublicationPolicy, setAtprotoChannelEnabled, uploadFile } from '../../api/client';
import { Dialog } from '../Dialog';
import { ExternalImage } from '../ExternalImage';

const EMPTY_ROLES: RoleInfo[] = [];
const EMPTY_CATEGORIES: CategoryInfo[] = [];
const EMPTY_CHANNELS: ChannelInfo[] = [];
const EMPTY_EMOJI: Record<string, { id: string; image_url: string }> = {};
const EMPTY_STICKERS: StickerInfo[] = [];
const EMPTY_FORUM_TAGS: Record<string, ForumTagInfo[]> = {};
const EMPTY_MEMBER_ROLES: Record<string, string[]> = {};

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
              className={`flex-1 rounded-md px-3 py-2 text-sm font-medium transition-colors ${
                visibleTab === t.key ? 'bg-bg-accent text-white' : 'text-text-muted hover:text-text-primary'
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

// ── Overview Tab ──────────────────────────────────────────

function OverviewTab({
  serverId,
  server,
  updateServer,
  setVanityCode,
  deleteServer,
  close,
}: {
  serverId: string;
  server: { name: string; icon_url?: string | null; role?: string | null };
  updateServer: (serverId: string, name?: string, iconUrl?: string) => Promise<void>;
  setVanityCode: (serverId: string, vanityCode?: string | null) => Promise<void>;
  deleteServer: (serverId: string) => Promise<void>;
  close: () => void;
}) {
  const [name, setName] = useState(server.name);
  const [iconUrl, setIconUrl] = useState(server.icon_url ?? '');
  const [vanity, setVanity] = useState('');
  const [saved, setSaved] = useState(false);
  const [pending, setPending] = useState<'details' | 'vanity' | 'delete' | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (key: 'details' | 'vanity' | 'delete', action: () => Promise<void>, accepted?: () => void) => {
    if (pending) return;
    setPending(key);
    setError(null);
    setSaved(false);
    try {
      await action();
      accepted?.();
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'The change was rejected.');
    } finally {
      setPending(null);
    }
  };

  const handleSave = () => {
    void run('details', () => updateServer(serverId, name.trim() || undefined, iconUrl.trim() || undefined));
  };

  const handleVanitySave = () => {
    void run('vanity', () => setVanityCode(serverId, vanity.trim() || null));
  };

  return (
    <div className="space-y-4">
      <div>
        <label className="mb-1 block text-sm font-medium text-text-secondary">Server Name</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary outline-none"
          placeholder="Server name"
        />
      </div>
      <div>
        <label className="mb-1 block text-sm font-medium text-text-secondary">Icon URL</label>
        <input
          type="text"
          value={iconUrl}
          onChange={(e) => setIconUrl(e.target.value)}
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
          placeholder="https://example.com/icon.png"
        />
        {iconUrl && (
          <div className="mt-2 flex items-center gap-3">
            <ExternalImage
              src={iconUrl}
              alt="Server icon preview"
              label="server icon preview"
              privacyScopeKey={`server-settings:${serverId}:icon-preview`}
              className="h-12 w-12 rounded-full object-cover"
            />
            <span className="text-xs text-text-muted">Preview</span>
          </div>
        )}
      </div>
      <div className="flex items-center gap-3">
        <button
          disabled={pending !== null}
          onClick={handleSave}
          className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
        >
          {pending === 'details' ? 'Saving…' : 'Save Changes'}
        </button>
        {saved && <span className="text-sm text-green-400">Saved!</span>}
      </div>

      {/* Vanity Invite URL */}
      <div className="border-t border-border-primary pt-4">
        <label className="mb-1 block text-sm font-medium text-text-secondary">Vanity Invite Code</label>
        <p className="mb-2 text-xs text-text-muted">Set a custom invite code (e.g., &quot;my-server&quot;). 2-32 lowercase letters, digits, and hyphens.</p>
        <div className="flex gap-2">
          <input
            type="text"
            value={vanity}
            onChange={(e) => setVanity(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ''))}
            className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            placeholder="my-server"
            maxLength={32}
          />
          <button
            disabled={pending !== null}
            onClick={handleVanitySave}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
          >
            {pending === 'vanity' ? 'Saving…' : 'Set'}
          </button>
          <button
            disabled={pending !== null}
            onClick={() => void run('vanity', () => setVanityCode(serverId, null), () => setVanity(''))}
            className="rounded px-3 py-2 text-sm text-text-muted hover:text-text-primary"
          >
            Clear
          </button>
        </div>
      </div>
      {error && <p role="alert" className="text-sm text-red-400">{error}</p>}
      {server.role === 'owner' && (
        <div className="border-t border-red-500/40 pt-4">
          <h3 className="mb-2 text-sm font-semibold text-red-400">Delete Server</h3>
          <p className="mb-2 text-xs text-text-muted">Permanently delete this server and its channels.</p>
          <button
            disabled={pending !== null}
            onClick={() => void run('delete', () => deleteServer(serverId), close)}
            className="rounded bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700"
          >
            {pending === 'delete' ? 'Deleting…' : 'Delete Server'}
          </button>
        </div>
      )}
    </div>
  );
}

// ── Channels Tab ─────────────────────────────────────────

function ChannelsTab({
  serverId,
  channels,
  categories,
  createChannel,
  deleteChannel,
  forumTags,
  createForumTag,
  updateForumTag,
  deleteForumTag,
  listForumTags,
  roles,
  channelPermissionOverrides,
  listChannelPermissionOverrides,
  setChannelPermissionOverride,
  deleteChannelPermissionOverride,
}: {
  serverId: string;
  channels: ChannelInfo[];
  categories: CategoryInfo[];
  createChannel: (serverId: string, name: string, categoryId?: string, isPrivate?: boolean, channelType?: 'text' | 'forum') => void;
  deleteChannel: (serverId: string, channel: string) => void;
  forumTags: Record<string, ForumTagInfo[]>;
  createForumTag: (serverId: string, channel: string, name: string, emoji: string | undefined, moderated: boolean) => void;
  updateForumTag: (serverId: string, channel: string, tag: ForumTagInfo) => void;
  deleteForumTag: (serverId: string, channel: string, tagId: string) => void;
  listForumTags: (serverId: string, channel: string) => void;
  roles: RoleInfo[];
  channelPermissionOverrides: Record<string, ChannelPermissionOverrideInfo[]>;
  listChannelPermissionOverrides: (serverId: string, channelId: string) => void;
  setChannelPermissionOverride: (serverId: string, channelId: string, targetType: 'role' | 'user', targetId: string, allowBits: number, denyBits: number) => void;
  deleteChannelPermissionOverride: (serverId: string, channelId: string, targetType: 'role' | 'user', targetId: string) => void;
}) {
  const [newName, setNewName] = useState('');
  const [newCategoryId, setNewCategoryId] = useState('');
  const [newPrivate, setNewPrivate] = useState(false);
  const [newType, setNewType] = useState<'text' | 'forum'>('text');
  const [permissionsChannelId, setPermissionsChannelId] = useState<string | null>(null);
  const [publicationEnabled, setPublicationEnabled] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let current = true;
    void Promise.all(channels.map(async (channel) => {
      try {
        return await getAtprotoChannelPublicationPolicy(channel.id);
      } catch {
        return null;
      }
    })).then((policies) => {
      if (!current) return;
      setPublicationEnabled(Object.fromEntries(policies.filter((policy) => policy?.eligible).map((policy) => [policy!.channel_id, policy!.channel_enabled])));
    });
    return () => { current = false; };
  }, [channels]);

  const sorted = [...channels].sort((a, b) => a.position - b.position);
  const sortedCategories = [...categories].sort((a, b) => a.position - b.position);

  const handleCreate = () => {
    if (!newName.trim()) return;
    createChannel(serverId, newName.trim(), newCategoryId || undefined, newPrivate || undefined, newType);
    setNewName('');
    setNewPrivate(false);
  };

  const getCategoryName = (id?: string | null) => {
    if (!id) return 'Uncategorized';
    return categories.find((c) => c.id === id)?.name ?? 'Unknown';
  };

  return (
    <div>
      {/* Create channel */}
      <div className="mb-4 space-y-2 rounded-md bg-bg-tertiary p-3">
        <div className="flex gap-2">
          <input
            type="text"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="New channel name"
            className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
          />
          <button
            onClick={handleCreate}
            disabled={!newName.trim()}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover disabled:opacity-50"
          >
            Create
          </button>
        </div>
        <div className="flex items-center gap-4">
          <select
            aria-label="Channel category"
            value={newCategoryId}
            onChange={(e) => setNewCategoryId(e.target.value)}
            className="rounded bg-bg-input px-2 py-1 text-sm text-text-primary outline-none"
          >
            <option value="">No Category</option>
            {sortedCategories.map((cat) => (
              <option key={cat.id} value={cat.id}>{cat.name}</option>
            ))}
          </select>
          <select
            aria-label="Channel type"
            value={newType}
            onChange={(e) => setNewType(e.target.value as 'text' | 'forum')}
            className="rounded bg-bg-input px-2 py-1 text-sm text-text-primary outline-none"
          >
            <option value="text">Text channel</option>
            <option value="forum">Forum channel</option>
          </select>
          <label className="flex items-center gap-1.5 text-sm text-text-secondary">
            <input
              type="checkbox"
              checked={newPrivate}
              onChange={(e) => setNewPrivate(e.target.checked)}
              className="rounded"
            />
            Private
          </label>
        </div>
      </div>

      {/* Channel list */}
      <div className="space-y-1">
        {sorted.map((ch) => (
          <div key={ch.id} aria-label={`Channel ${ch.name.replace(/^#/, '')}`} className="rounded-md bg-bg-tertiary px-3 py-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
              {ch.is_private ? (
                <svg className="h-4 w-4 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                </svg>
              ) : (
                <span className="text-text-muted">#</span>
              )}
              <span className="text-sm font-medium text-text-primary">{ch.name.replace(/^#/, '')}</span>
              {ch.channel_type === 'forum' && <span className="rounded bg-bg-accent/20 px-1 text-xs text-text-secondary">Forum</span>}
              <span className="text-xs text-text-muted">{getCategoryName(ch.category_id)}</span>
              {ch.is_nsfw && <span className="rounded bg-red-500/20 px-1 text-xs text-red-400">NSFW</span>}
              </div>
              <button
                onClick={() => setPermissionsChannelId((current) => current === ch.id ? null : ch.id)}
                aria-expanded={permissionsChannelId === ch.id}
                className="rounded px-2 py-1 text-xs text-text-muted hover:text-text-primary"
              >
                Permissions
              </button>
              <button
                onClick={() => deleteChannel(serverId, ch.name)}
                className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
              >
                Delete
              </button>
              {!ch.is_private && !ch.thread_parent_message_id && (
                <button
                  type="button"
                  onClick={async () => {
                    const enabled = !publicationEnabled[ch.id];
                    await setAtprotoChannelEnabled(ch.id, enabled);
                    setPublicationEnabled((current) => ({ ...current, [ch.id]: enabled }));
                  }}
                  className="text-xs text-text-muted hover:text-text-primary"
                >
                  {publicationEnabled[ch.id] ? 'Disable AT publication' : 'Enable AT publication'}
                </button>
              )}
            </div>
            {permissionsChannelId === ch.id && (
              <ChannelPermissionEditor
                serverId={serverId}
                channel={ch}
                roles={roles}
                overrides={channelPermissionOverrides[ch.id] ?? []}
                load={listChannelPermissionOverrides}
                save={setChannelPermissionOverride}
                remove={deleteChannelPermissionOverride}
              />
            )}
            {ch.channel_type === 'forum' && (
              <ForumTagEditor
                serverId={serverId}
                channel={ch.name}
                tags={forumTags[channelKey(serverId, ch.name)] ?? []}
                createTag={createForumTag}
                updateTag={updateForumTag}
                deleteTag={deleteForumTag}
                listTags={listForumTags}
              />
            )}
          </div>
        ))}

        {sorted.length === 0 && (
          <p className="py-4 text-center text-sm text-text-muted">No channels yet.</p>
        )}
      </div>
    </div>
  );
}

type PermissionDecision = 'inherit' | 'allow' | 'deny';

const channelPermissionChoices = [
  { flag: Permissions.VIEW_CHANNELS, label: 'View channel' },
  { flag: Permissions.SEND_MESSAGES, label: 'Send messages' },
  { flag: Permissions.READ_MESSAGE_HISTORY, label: 'Read message history' },
  { flag: Permissions.EMBED_LINKS, label: 'Embed links' },
  { flag: Permissions.ATTACH_FILES, label: 'Attach files' },
  { flag: Permissions.ADD_REACTIONS, label: 'Add reactions' },
  { flag: Permissions.MENTION_EVERYONE, label: 'Mention everyone' },
  { flag: Permissions.MANAGE_MESSAGES, label: 'Manage messages' },
  { flag: Permissions.MANAGE_CHANNELS, label: 'Manage channel' },
] as const;

function ChannelPermissionEditor({
  serverId,
  channel,
  roles,
  overrides,
  load,
  save,
  remove,
}: {
  serverId: string;
  channel: ChannelInfo;
  roles: RoleInfo[];
  overrides: ChannelPermissionOverrideInfo[];
  load: (serverId: string, channelId: string) => void;
  save: (serverId: string, channelId: string, targetType: 'role' | 'user', targetId: string, allowBits: number, denyBits: number) => void;
  remove: (serverId: string, channelId: string, targetType: 'role' | 'user', targetId: string) => void;
}) {
  const [targetType, setTargetType] = useState<'role' | 'user'>('role');
  const [targetId, setTargetId] = useState(roles[0]?.id ?? '');
  const [decisions, setDecisions] = useState<Record<number, PermissionDecision>>({});
  const selectedTargetId = targetType === 'role' ? targetId || roles[0]?.id || '' : targetId;
  const selectedTargetLabel = targetType === 'role'
    ? roles.find((role) => role.id === selectedTargetId)?.name ?? selectedTargetId
    : selectedTargetId;
  const current = overrides.find((item) => item.target_type === targetType && item.target_id === selectedTargetId);

  useEffect(() => {
    load(serverId, channel.id);
  }, [serverId, channel.id, load]);

  const decisionFor = (flag: number): PermissionDecision => decisions[flag]
    ?? (current && (current.allow_bits & flag) !== 0
      ? 'allow'
      : current && (current.deny_bits & flag) !== 0
        ? 'deny'
        : 'inherit');

  const submit = () => {
    if (!selectedTargetId.trim()) return;
    let allowBits = 0;
    let denyBits = 0;
    for (const { flag } of channelPermissionChoices) {
      if (decisionFor(flag) === 'allow') allowBits |= flag;
      if (decisionFor(flag) === 'deny') denyBits |= flag;
    }
    if (allowBits === 0 && denyBits === 0) {
      if (current) remove(serverId, channel.id, targetType, selectedTargetId.trim());
      return;
    }
    save(serverId, channel.id, targetType, selectedTargetId.trim(), allowBits, denyBits);
  };

  return (
    <section aria-label={`Permissions for ${channel.name.replace(/^#/, '')}`} className="mt-3 border-t border-border-primary pt-3">
      <div className="mb-2 flex gap-2">
        <select
          aria-label="Override target type"
          value={targetType}
          onChange={(event) => {
            const next = event.target.value as 'role' | 'user';
            setTargetType(next);
            setTargetId(next === 'role' ? roles[0]?.id ?? '' : '');
            setDecisions({});
          }}
          className="rounded bg-bg-input px-2 py-1 text-sm text-text-primary"
        >
          <option value="role">Role</option>
          <option value="user">Member</option>
        </select>
        {targetType === 'role' ? (
          <select
            aria-label="Override role"
            value={selectedTargetId}
            onChange={(event) => {
              setTargetId(event.target.value);
              setDecisions({});
            }}
            className="min-w-0 flex-1 rounded bg-bg-input px-2 py-1 text-sm text-text-primary"
          >
            {roles.map((role) => <option key={role.id} value={role.id}>{role.name}</option>)}
          </select>
        ) : (
          <input
            aria-label="Override member user ID"
            value={targetId}
            onChange={(event) => {
              setTargetId(event.target.value);
              setDecisions({});
            }}
            placeholder="Member user ID"
            className="min-w-0 flex-1 rounded bg-bg-input px-2 py-1 text-sm text-text-primary"
          />
        )}
      </div>
      <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
        {channelPermissionChoices.map(({ flag, label }) => (
          <label key={flag} className="flex items-center justify-between gap-2 text-xs text-text-secondary">
            {label}
            <select
              aria-label={`${label} for ${selectedTargetLabel}`}
              value={decisionFor(flag)}
              onChange={(event) => setDecisions((previous) => ({
                ...previous,
                [flag]: event.target.value as PermissionDecision,
              }))}
              className="rounded bg-bg-input px-1 py-0.5 text-xs text-text-primary"
            >
              <option value="inherit">Inherit</option>
              <option value="allow">Allow</option>
              <option value="deny">Deny</option>
            </select>
          </label>
        ))}
      </div>
      <div className="mt-3 flex gap-2">
        <button
          onClick={submit}
          disabled={!selectedTargetId.trim()}
          className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white disabled:opacity-50"
        >
          Save permissions
        </button>
        {current && (
          <button
            onClick={() => {
              remove(serverId, channel.id, targetType, selectedTargetId);
              setDecisions({});
            }}
            className="rounded px-3 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
          >
            Reset to inherited
          </button>
        )}
      </div>
    </section>
  );
}

function ForumTagEditor({
  serverId,
  channel,
  tags,
  createTag,
  updateTag,
  deleteTag,
  listTags,
}: {
  serverId: string;
  channel: string;
  tags: ForumTagInfo[];
  createTag: (serverId: string, channel: string, name: string, emoji: string | undefined, moderated: boolean) => void;
  updateTag: (serverId: string, channel: string, tag: ForumTagInfo) => void;
  deleteTag: (serverId: string, channel: string, tagId: string) => void;
  listTags: (serverId: string, channel: string) => void;
}) {
  const [name, setName] = useState('');
  const [emoji, setEmoji] = useState('');
  const [moderated, setModerated] = useState(false);

  useEffect(() => {
    listTags(serverId, channel);
  }, [serverId, channel, listTags]);

  const addTag = () => {
    if (!name.trim()) return;
    createTag(serverId, channel, name.trim(), emoji.trim() || undefined, moderated);
    setName('');
    setEmoji('');
    setModerated(false);
  };

  return (
    <div className="mt-2 border-t border-border-primary pt-2">
      <div className="mb-2 text-xs font-semibold uppercase text-text-muted">Forum tags</div>
      <div className="space-y-1">
        {[...tags].sort((a, b) => a.position - b.position).map((tag) => (
          <div key={tag.id} className="flex items-center gap-2">
            <input
              aria-label={`Tag name for ${tag.name}`}
              defaultValue={tag.name}
              maxLength={20}
              onBlur={(event) => {
                const nextName = event.target.value.trim();
                if (nextName && nextName !== tag.name) updateTag(serverId, channel, { ...tag, name: nextName });
              }}
              className="min-w-0 flex-1 rounded bg-bg-input px-2 py-1 text-xs text-text-primary outline-none"
            />
            <input
              aria-label={`Tag emoji for ${tag.name}`}
              defaultValue={tag.emoji ?? ''}
              maxLength={16}
              onBlur={(event) => {
                const nextEmoji = event.target.value.trim() || null;
                if (nextEmoji !== (tag.emoji ?? null)) updateTag(serverId, channel, { ...tag, emoji: nextEmoji });
              }}
              className="w-16 rounded bg-bg-input px-2 py-1 text-center text-xs text-text-primary outline-none"
              placeholder="Emoji"
            />
            <label className="flex items-center gap-1 text-xs text-text-secondary">
              <input
                type="checkbox"
                checked={tag.moderated}
                onChange={(event) => updateTag(serverId, channel, { ...tag, moderated: event.target.checked })}
              />
              Moderated
            </label>
            <button
              onClick={() => deleteTag(serverId, channel, tag.id)}
              className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
            >
              Delete
            </button>
          </div>
        ))}
      </div>
      <div className="mt-2 flex items-center gap-2">
        <input
          aria-label={`New tag name for ${channel}`}
          value={name}
          maxLength={20}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => event.key === 'Enter' && addTag()}
          className="min-w-0 flex-1 rounded bg-bg-input px-2 py-1 text-xs text-text-primary outline-none"
          placeholder="New tag"
        />
        <input
          aria-label={`New tag emoji for ${channel}`}
          value={emoji}
          maxLength={16}
          onChange={(event) => setEmoji(event.target.value)}
          className="w-16 rounded bg-bg-input px-2 py-1 text-center text-xs text-text-primary outline-none"
          placeholder="Emoji"
        />
        <label className="flex items-center gap-1 text-xs text-text-secondary">
          <input type="checkbox" checked={moderated} onChange={(event) => setModerated(event.target.checked)} />
          Moderated
        </label>
        <button
          onClick={addTag}
          disabled={!name.trim() || tags.length >= 20}
          className="rounded bg-bg-accent px-2 py-1 text-xs font-medium text-white disabled:opacity-50"
        >
          Add tag
        </button>
      </div>
    </div>
  );
}

// ── Roles Tab ────────────────────────────────────────────

function RolesTab({
  serverId,
  roles,
  createRole,
  updateRole,
  deleteRole,
  assignRole,
  removeRole,
  memberRoles,
}: {
  serverId: string;
  roles: RoleInfo[];
  createRole: (serverId: string, name: string, color?: string, permissions?: number) => void;
  updateRole: (serverId: string, roleId: string, updates: { name?: string; color?: string; permissions?: number; position?: number }) => void;
  deleteRole: (serverId: string, roleId: string) => void;
  assignRole: (serverId: string, userId: string, roleId: string) => void;
  removeRole: (serverId: string, userId: string, roleId: string) => void;
  memberRoles: Record<string, string[]>;
}) {
  const [newName, setNewName] = useState('');
  const [newColor, setNewColor] = useState('#99aab5');
  const [editingRole, setEditingRole] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [editColor, setEditColor] = useState('');
  const [memberId, setMemberId] = useState('');

  const sortedRoles = [...roles].sort((a, b) => b.position - a.position);

  const handleCreate = () => {
    if (!newName.trim()) return;
    createRole(serverId, newName.trim(), newColor);
    setNewName('');
  };

  const startEdit = (role: RoleInfo) => {
    setEditingRole(role.id);
    setEditName(role.name);
    setEditColor(role.color || '#99aab5');
  };

  const saveEdit = (roleId: string) => {
    updateRole(serverId, roleId, { name: editName.trim() || undefined, color: editColor });
    setEditingRole(null);
  };

  const permissionLabels: { flag: number; label: string }[] = [
    { flag: Permissions.MANAGE_CHANNELS, label: 'Manage Channels' },
    { flag: Permissions.MANAGE_ROLES, label: 'Manage Roles' },
    { flag: Permissions.MANAGE_SERVER, label: 'Manage Server' },
    { flag: Permissions.MANAGE_MESSAGES, label: 'Manage Messages' },
    { flag: Permissions.KICK_MEMBERS, label: 'Kick Members' },
    { flag: Permissions.BAN_MEMBERS, label: 'Ban Members' },
    { flag: Permissions.MENTION_EVERYONE, label: 'Mention Everyone' },
    { flag: Permissions.ADMINISTRATOR, label: 'Administrator' },
  ];

  return (
    <div>
      {/* Create role */}
      <div className="mb-4 flex gap-2">
        <input
          type="text"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          placeholder="New role name"
          className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
          onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
        />
        <input
          type="color"
          aria-label="New role color"
          value={newColor}
          onChange={(e) => setNewColor(e.target.value)}
          className="h-9 w-9 cursor-pointer rounded border-0 bg-transparent"
        />
        <button
          onClick={handleCreate}
          className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
        >
          Create
        </button>
      </div>

      {/* Role list */}
      <div className="space-y-2">
        {sortedRoles.map((role) => (
          <div key={role.id} aria-label={`Role ${role.name}`} className="rounded-md bg-bg-tertiary p-3">
            {editingRole === role.id ? (
              <div className="space-y-3">
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={editName}
                    onChange={(e) => setEditName(e.target.value)}
                    className="flex-1 rounded bg-bg-input px-3 py-1.5 text-sm text-text-primary outline-none"
                  />
                  <input
                    type="color"
                    aria-label={`Color for ${role.name}`}
                    value={editColor}
                    onChange={(e) => setEditColor(e.target.value)}
                    className="h-8 w-8 cursor-pointer rounded border-0 bg-transparent"
                  />
                </div>

                {/* Permission toggles */}
                <div className="grid grid-cols-2 gap-2">
                  {permissionLabels.map(({ flag, label }) => {
                    const has = hasPermission(role.permissions, flag);
                    return (
                      <label key={flag} className="flex items-center gap-2 text-sm text-text-secondary">
                        <input
                          type="checkbox"
                          checked={has}
                          onChange={() => {
                            const newPerms = has ? (role.permissions & ~flag) : (role.permissions | flag);
                            updateRole(serverId, role.id, { permissions: newPerms });
                          }}
                          className="rounded"
                          disabled={role.is_default && role.name === '@everyone'}
                        />
                        {label}
                      </label>
                    );
                  })}
                </div>

                <div className="flex gap-2">
                  <button
                    onClick={() => saveEdit(role.id)}
                    className="rounded bg-bg-accent px-3 py-1 text-sm text-white hover:bg-bg-accent-hover"
                  >
                    Save
                  </button>
                  <button
                    onClick={() => setEditingRole(null)}
                    className="rounded px-3 py-1 text-sm text-text-muted hover:text-text-primary"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            ) : (
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <div
                    className="h-3 w-3 rounded-full"
                    style={{ backgroundColor: role.color || '#99aab5' }}
                  />
                  <span className="text-sm font-medium text-text-primary">{role.name}</span>
                  {role.is_default && (
                    <span className="rounded bg-bg-primary px-1.5 py-0.5 text-xs text-text-muted">default</span>
                  )}
                  <span className="text-xs text-text-muted">pos: {role.position}</span>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => startEdit(role)}
                    className="rounded px-2 py-1 text-xs text-text-muted hover:text-text-primary"
                  >
                    Edit
                  </button>
                  {!role.is_default && (
                    <button
                      onClick={() => deleteRole(serverId, role.id)}
                      className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
                    >
                      Delete
                    </button>
                  )}
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      <div className="mt-5 border-t border-border-primary pt-4">
        <h3 className="mb-2 text-sm font-semibold text-text-secondary">Member Role Assignments</h3>
        <input
          value={memberId}
          onChange={(event) => setMemberId(event.target.value)}
          placeholder="Member user ID"
          className="mb-2 w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary outline-none"
        />
        {memberId.trim() && (
          <div className="space-y-1">
            {sortedRoles.filter((role) => !role.is_default).map((role) => {
              const assigned = (memberRoles[memberId.trim()] ?? []).includes(role.id);
              return (
                <label key={role.id} className="flex items-center justify-between rounded bg-bg-tertiary px-3 py-2 text-sm text-text-secondary">
                  <span style={{ color: role.color ?? undefined }}>{role.name}</span>
                  <input
                    aria-label={`${assigned ? 'Remove' : 'Assign'} ${role.name} for ${memberId.trim()}`}
                    type="checkbox"
                    checked={assigned}
                    onChange={() => assigned
                      ? removeRole(serverId, memberId.trim(), role.id)
                      : assignRole(serverId, memberId.trim(), role.id)}
                  />
                </label>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

// ── Categories Tab ───────────────────────────────────────

function CategoriesTab({
  serverId,
  categories,
  createCategory,
  updateCategory,
  deleteCategory,
}: {
  serverId: string;
  categories: CategoryInfo[];
  createCategory: (serverId: string, name: string) => void;
  updateCategory: (serverId: string, categoryId: string, updates: { name?: string; position?: number }) => void;
  deleteCategory: (serverId: string, categoryId: string) => void;
}) {
  const [newName, setNewName] = useState('');
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState('');

  const sorted = [...categories].sort((a, b) => a.position - b.position);

  const handleCreate = () => {
    if (!newName.trim()) return;
    createCategory(serverId, newName.trim());
    setNewName('');
  };

  const startEdit = (cat: CategoryInfo) => {
    setEditingId(cat.id);
    setEditName(cat.name);
  };

  const saveEdit = (catId: string) => {
    if (editName.trim()) {
      updateCategory(serverId, catId, { name: editName.trim() });
    }
    setEditingId(null);
  };

  return (
    <div>
      {/* Create category */}
      <div className="mb-4 flex gap-2">
        <input
          type="text"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          placeholder="New category name"
          className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
          onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
        />
        <button
          onClick={handleCreate}
          className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
        >
          Create
        </button>
      </div>

      {/* Category list */}
      <div className="space-y-2">
        {sorted.map((cat) => (
          <div key={cat.id} className="flex items-center justify-between rounded-md bg-bg-tertiary p-3">
            {editingId === cat.id ? (
              <div className="flex flex-1 gap-2">
                <input
                  type="text"
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  className="flex-1 rounded bg-bg-input px-3 py-1.5 text-sm text-text-primary outline-none"
                  onKeyDown={(e) => e.key === 'Enter' && saveEdit(cat.id)}
                />
                <button
                  onClick={() => saveEdit(cat.id)}
                  className="rounded bg-bg-accent px-3 py-1 text-sm text-white hover:bg-bg-accent-hover"
                >
                  Save
                </button>
                <button
                  onClick={() => setEditingId(null)}
                  className="rounded px-3 py-1 text-sm text-text-muted hover:text-text-primary"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <>
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-text-primary">{cat.name}</span>
                  <span className="text-xs text-text-muted">pos: {cat.position}</span>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => startEdit(cat)}
                    className="rounded px-2 py-1 text-xs text-text-muted hover:text-text-primary"
                  >
                    Edit
                  </button>
                  <button
                    onClick={() => deleteCategory(serverId, cat.id)}
                    className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
                  >
                    Delete
                  </button>
                </div>
              </>
            )}
          </div>
        ))}

        {sorted.length === 0 && (
          <p className="py-4 text-center text-sm text-text-muted">No categories yet. Create one to organize your channels.</p>
        )}
      </div>
    </div>
  );
}

// ── Emoji Tab ────────────────────────────────────────────

function EmojiTab({
  serverId,
  emoji,
  createEmoji,
  deleteEmoji,
}: {
  serverId: string;
  emoji: Record<string, { id: string; image_url: string }>;
  createEmoji: (serverId: string, name: string, imageUrl: string) => Promise<void>;
  deleteEmoji: (serverId: string, emojiId: string) => Promise<void>;
}) {
  const [newName, setNewName] = useState('');
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState('');
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);

  const emojiEntries = Object.entries(emoji);

  const handleUpload = async () => {
    const file = selectedFile;
    if (!file || !newName.trim()) return;

    // Validate: images only, max 256KB
    if (!file.type.startsWith('image/')) {
      setError('Only image files are allowed');
      return;
    }
    if (file.size > 256 * 1024) {
      setError('Emoji must be under 256KB');
      return;
    }

    setUploading(true);
    setError('');
    try {
      const attachment = await uploadFile(file, { serverId, purpose: 'emoji' });
      await createEmoji(serverId, newName.trim(), attachment.url);
      setNewName('');
      setSelectedFile(null);
      if (fileInputRef.current) fileInputRef.current.value = '';
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Upload failed');
    } finally {
      setUploading(false);
    }
  };

  return (
    <div>
      {/* Upload form */}
      <div className="mb-4 space-y-2 rounded-md bg-bg-tertiary p-3">
        <div className="flex gap-2">
          <input
            type="text"
            value={newName}
            onChange={(e) => setNewName(e.target.value.toLowerCase().replace(/[^a-z0-9_]/g, ''))}
            placeholder="emoji_name"
            className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            maxLength={32}
          />
          <button
            onClick={handleUpload}
            disabled={uploading || !newName.trim() || !selectedFile}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover disabled:opacity-50"
          >
            {uploading ? 'Uploading...' : 'Upload'}
          </button>
        </div>
        <input
          ref={fileInputRef}
          onChange={(event) => setSelectedFile(event.target.files?.[0] ?? null)}
          type="file"
          accept="image/*"
          className="text-sm text-text-secondary file:mr-3 file:rounded file:border-0 file:bg-bg-accent file:px-3 file:py-1 file:text-sm file:text-white"
        />
        {newName && <p className="text-xs text-text-muted">Will be used as <code className="rounded bg-bg-primary px-1">:{newName}:</code></p>}
        {error && <p className="text-xs text-red-400">{error}</p>}
      </div>

      {/* Emoji list */}
      <div className="space-y-1">
        {emojiEntries.map(([name, emoji]) => (
          <div key={name} className="flex items-center justify-between rounded-md bg-bg-tertiary px-3 py-2">
            <div className="flex items-center gap-3">
              <img src={emoji.image_url} alt={name} className="h-8 w-8 object-contain" />
              <span className="text-sm text-text-primary">:{name}:</span>
            </div>
            <button
              onClick={() => deleteEmoji(serverId, emoji.id)}
              aria-label={`Delete emoji ${name}`}
              className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
            >
              Delete
            </button>
          </div>
        ))}

        {emojiEntries.length === 0 && (
          <p className="py-4 text-center text-sm text-text-muted">No custom emoji yet. Upload one above!</p>
        )}
      </div>
    </div>
  );
}

// ── Stickers Tab ──────────────────────────────────────────

function StickersTab({
  serverId,
  stickers,
  createSticker,
  deleteSticker,
}: {
  serverId: string;
  stickers: StickerInfo[];
  createSticker: (serverId: string, name: string, imageUrl: string, description?: string) => Promise<void>;
  deleteSticker: (serverId: string, stickerId: string) => Promise<void>;
}) {
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState('');
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);

  const handleUpload = async () => {
    const file = selectedFile;
    if (!file || !newName.trim()) return;

    if (!file.type.startsWith('image/')) {
      setError('Only image files are allowed');
      return;
    }
    if (file.size > 512 * 1024) {
      setError('Sticker must be under 512KB');
      return;
    }

    setUploading(true);
    setError('');
    try {
      const attachment = await uploadFile(file, { serverId, purpose: 'sticker' });
      await createSticker(serverId, newName.trim(), attachment.url, newDesc.trim() || undefined);
      setNewName('');
      setNewDesc('');
      setSelectedFile(null);
      if (fileInputRef.current) fileInputRef.current.value = '';
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Upload failed');
    } finally {
      setUploading(false);
    }
  };

  return (
    <div>
      {/* Upload form */}
      <div className="mb-4 space-y-2 rounded-md bg-bg-tertiary p-3">
        <div className="flex gap-2">
          <input
            type="text"
            value={newName}
            onChange={(e) => setNewName(e.target.value.toLowerCase().replace(/[^a-z0-9_]/g, ''))}
            placeholder="sticker_name"
            className="flex-1 rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
            maxLength={32}
          />
          <button
            onClick={handleUpload}
            disabled={uploading || !newName.trim() || !selectedFile}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover disabled:opacity-50"
          >
            {uploading ? 'Uploading...' : 'Upload'}
          </button>
        </div>
        <input
          type="text"
          value={newDesc}
          onChange={(e) => setNewDesc(e.target.value)}
          placeholder="Description (optional)"
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary placeholder-text-muted outline-none"
          maxLength={100}
        />
        <input
          ref={fileInputRef}
          onChange={(event) => setSelectedFile(event.target.files?.[0] ?? null)}
          type="file"
          accept="image/*"
          className="text-sm text-text-secondary file:mr-3 file:rounded file:border-0 file:bg-bg-accent file:px-3 file:py-1 file:text-sm file:text-white"
        />
        {newName && <p className="text-xs text-text-muted">Send as <code className="rounded bg-bg-primary px-1">[sticker:{newName}]</code></p>}
        {error && <p className="text-xs text-red-400">{error}</p>}
      </div>

      {/* Sticker list */}
      <div className="space-y-1">
        {stickers.map((sticker) => (
          <div key={sticker.id} className="flex items-center justify-between rounded-md bg-bg-tertiary px-3 py-2">
            <div className="flex items-center gap-3">
              <img src={sticker.image_url} alt={sticker.name} className="h-12 w-12 object-contain" />
              <div>
                <span className="text-sm font-medium text-text-primary">{sticker.name}</span>
                {sticker.description && (
                  <p className="text-xs text-text-muted">{sticker.description}</p>
                )}
              </div>
            </div>
            <button
              onClick={() => deleteSticker(serverId, sticker.id)}
              aria-label={`Delete sticker ${sticker.name}`}
              className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
            >
              Delete
            </button>
          </div>
        ))}

        {stickers.length === 0 && (
          <p className="py-4 text-center text-sm text-text-muted">No stickers yet. Upload one above!</p>
        )}
      </div>
    </div>
  );
}
