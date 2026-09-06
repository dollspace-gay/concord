import { useRef, useState } from 'react';
import { uploadFile } from '../../../api/client';

// ── Emoji Tab ────────────────────────────────────────────

export function EmojiTab({
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
