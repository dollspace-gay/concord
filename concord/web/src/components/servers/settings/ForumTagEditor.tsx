import { useEffect, useState } from 'react';
import type { ForumTagInfo } from '../../../api/types';

export function ForumTagEditor({
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
