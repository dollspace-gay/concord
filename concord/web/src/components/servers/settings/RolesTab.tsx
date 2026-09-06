import { useState } from 'react';
import type { RoleInfo } from '../../../api/types';
import { hasPermission, Permissions } from '../../../api/types';

// ── Roles Tab ────────────────────────────────────────────

export function RolesTab({
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
