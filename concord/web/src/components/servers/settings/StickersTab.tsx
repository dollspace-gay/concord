import { useRef, useState } from 'react';
import { uploadFile } from '../../../api/client';
import type { StickerInfo } from '../../../api/types';

// ── Stickers Tab ──────────────────────────────────────────

export function StickersTab({
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
