import { useState } from 'react';
import type { CategoryInfo } from '../../../api/types';

// ── Categories Tab ───────────────────────────────────────

export function CategoriesTab({
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
