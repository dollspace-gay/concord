import { useEffect, useState } from 'react';
import { getAtprotoChannelPublicationPolicy, setAtprotoChannelEnabled } from '../../../api/client';
import type { ChannelPermissionOverrideInfo } from '../../../api/generated/contract';
import type { CategoryInfo, ChannelInfo, ForumTagInfo, RoleInfo } from '../../../api/types';
import { channelKey } from '../../../api/types';
import { ChannelPermissionEditor } from './ChannelPermissionEditor';
import { ForumTagEditor } from './ForumTagEditor';

// ── Channels Tab ─────────────────────────────────────────

export function ChannelsTab({
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
