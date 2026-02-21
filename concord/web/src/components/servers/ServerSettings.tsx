import { useState, useEffect, useRef } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import type { RoleInfo, CategoryInfo, ChannelInfo, StickerInfo } from '../../api/types';
import { hasPermission, Permissions } from '../../api/types';
import { uploadFile } from '../../api/client';

const EMPTY_ROLES: RoleInfo[] = [];
const EMPTY_CATEGORIES: CategoryInfo[] = [];
const EMPTY_CHANNELS: ChannelInfo[] = [];
const EMPTY_EMOJI: Record<string, { id: string; image_url: string }> = {};
const EMPTY_STICKERS: StickerInfo[] = [];

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
  const createCategory = useChatStore((s) => s.createCategory);
  const updateCategory = useChatStore((s) => s.updateCategory);
  const deleteCategory = useChatStore((s) => s.deleteCategory);
  const updateServer = useChatStore((s) => s.updateServer);
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

  const tabs: { key: Tab; label: string }[] = [
    { key: 'overview', label: 'Overview' },
    { key: 'channels', label: 'Channels' },
    { key: 'roles', label: 'Roles' },
    { key: 'categories', label: 'Categories' },
    { key: 'emoji', label: 'Emoji' },
    { key: 'stickers', label: 'Stickers' },
  ];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="max-h-[80vh] w-full max-w-2xl overflow-y-auto rounded-lg bg-bg-secondary p-6">
        <div className="mb-6 flex items-center justify-between">
          <h2 className="text-xl font-bold text-text-primary">{serverName} Settings</h2>
          <button
            onClick={() => setShowServerSettings(false)}
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
                tab === t.key ? 'bg-bg-accent text-white' : 'text-text-muted hover:text-text-primary'
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>

        {tab === 'overview' && (
          <OverviewTab serverId={activeServer} server={server!} updateServer={updateServer} setVanityCode={setVanityCode} />
        )}
        {tab === 'channels' && (
          <ChannelsTab
            serverId={activeServer}
            channels={channels}
            categories={categories}
            createChannel={createChannel}
            deleteChannel={deleteChannel}
          />
        )}
        {tab === 'roles' && (
          <RolesTab
            serverId={activeServer}
            roles={roles}
            createRole={createRole}
            updateRole={updateRole}
            deleteRole={deleteRole}
          />
        )}
        {tab === 'categories' && (
          <CategoriesTab
            serverId={activeServer}
            categories={categories}
            createCategory={createCategory}
            updateCategory={updateCategory}
            deleteCategory={deleteCategory}
          />
        )}
        {tab === 'emoji' && (
          <EmojiTab
            serverId={activeServer}
            emoji={customEmoji}
            createEmoji={createEmoji}
            deleteEmoji={deleteEmoji}
          />
        )}
        {tab === 'stickers' && (
          <StickersTab
            serverId={activeServer}
            stickers={stickers}
            createSticker={createSticker}
            deleteSticker={deleteSticker}
          />
        )}
      </div>
    </div>
  );
}

// ── Overview Tab ──────────────────────────────────────────

function OverviewTab({
  serverId,
  server,
  updateServer,
  setVanityCode,
}: {
  serverId: string;
  server: { name: string; icon_url?: string | null };
  updateServer: (serverId: string, name?: string, iconUrl?: string) => void;
  setVanityCode: (serverId: string, vanityCode?: string | null) => void;
}) {
  const [name, setName] = useState(server.name);
  const [iconUrl, setIconUrl] = useState(server.icon_url ?? '');
  const [vanity, setVanity] = useState('');
  const [saved, setSaved] = useState(false);

  const handleSave = () => {
    updateServer(serverId, name.trim() || undefined, iconUrl.trim() || undefined);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const handleVanitySave = () => {
    setVanityCode(serverId, vanity.trim() || null);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
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
            <img
              src={iconUrl}
              alt="Server icon preview"
              className="h-12 w-12 rounded-full object-cover"
              onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
            />
            <span className="text-xs text-text-muted">Preview</span>
          </div>
        )}
      </div>
      <div className="flex items-center gap-3">
        <button
          onClick={handleSave}
          className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
        >
          Save Changes
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
            onClick={handleVanitySave}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover"
          >
            Set
          </button>
          <button
            onClick={() => { setVanityCode(serverId, null); setVanity(''); }}
            className="rounded px-3 py-2 text-sm text-text-muted hover:text-text-primary"
          >
            Clear
          </button>
        </div>
      </div>
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
}: {
  serverId: string;
  channels: ChannelInfo[];
  categories: CategoryInfo[];
  createChannel: (serverId: string, name: string, categoryId?: string, isPrivate?: boolean) => void;
  deleteChannel: (serverId: string, channel: string) => void;
}) {
  const [newName, setNewName] = useState('');
  const [newCategoryId, setNewCategoryId] = useState('');
  const [newPrivate, setNewPrivate] = useState(false);

  const sorted = [...channels].sort((a, b) => a.position - b.position);
  const sortedCategories = [...categories].sort((a, b) => a.position - b.position);

  const handleCreate = () => {
    if (!newName.trim()) return;
    createChannel(serverId, newName.trim(), newCategoryId || undefined, newPrivate || undefined);
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
            value={newCategoryId}
            onChange={(e) => setNewCategoryId(e.target.value)}
            className="rounded bg-bg-input px-2 py-1 text-sm text-text-primary outline-none"
          >
            <option value="">No Category</option>
            {sortedCategories.map((cat) => (
              <option key={cat.id} value={cat.id}>{cat.name}</option>
            ))}
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
          <div key={ch.id} className="flex items-center justify-between rounded-md bg-bg-tertiary px-3 py-2">
            <div className="flex items-center gap-2">
              {ch.is_private ? (
                <svg className="h-4 w-4 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                </svg>
              ) : (
                <span className="text-text-muted">#</span>
              )}
              <span className="text-sm font-medium text-text-primary">{ch.name.replace(/^#/, '')}</span>
              <span className="text-xs text-text-muted">{getCategoryName(ch.category_id)}</span>
              {ch.is_nsfw && <span className="rounded bg-red-500/20 px-1 text-xs text-red-400">NSFW</span>}
            </div>
            <button
              onClick={() => deleteChannel(serverId, ch.name)}
              className="rounded px-2 py-1 text-xs text-bg-danger hover:bg-bg-danger/10"
            >
              Delete
            </button>
          </div>
        ))}

        {sorted.length === 0 && (
          <p className="py-4 text-center text-sm text-text-muted">No channels yet.</p>
        )}
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
}: {
  serverId: string;
  roles: RoleInfo[];
  createRole: (serverId: string, name: string, color?: string, permissions?: number) => void;
  updateRole: (serverId: string, roleId: string, updates: { name?: string; color?: string; permissions?: number; position?: number }) => void;
  deleteRole: (serverId: string, roleId: string) => void;
}) {
  const [newName, setNewName] = useState('');
  const [newColor, setNewColor] = useState('#99aab5');
  const [editingRole, setEditingRole] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [editColor, setEditColor] = useState('');

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
          <div key={role.id} className="rounded-md bg-bg-tertiary p-3">
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

  const emojiEntries = Object.entries(emoji);

  const handleUpload = async () => {
    const file = fileInputRef.current?.files?.[0];
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
      const attachment = await uploadFile(file);
      await createEmoji(serverId, newName.trim(), attachment.url);
      setNewName('');
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
            disabled={uploading || !newName.trim() || !fileInputRef.current?.files?.length}
            className="rounded bg-bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-bg-accent-hover disabled:opacity-50"
          >
            {uploading ? 'Uploading...' : 'Upload'}
          </button>
        </div>
        <input
          ref={fileInputRef}
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

  const handleUpload = async () => {
    const file = fileInputRef.current?.files?.[0];
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
      const attachment = await uploadFile(file);
      await createSticker(serverId, newName.trim(), attachment.url, newDesc.trim() || undefined);
      setNewName('');
      setNewDesc('');
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
            disabled={uploading || !newName.trim() || !fileInputRef.current?.files?.length}
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
