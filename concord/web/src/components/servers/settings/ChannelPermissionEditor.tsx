import { useEffect, useState } from 'react';
import type { ChannelPermissionOverrideInfo } from '../../../api/generated/contract';
import type { ChannelInfo, RoleInfo } from '../../../api/types';
import { Permissions } from '../../../api/types';

export type PermissionDecision = 'inherit' | 'allow' | 'deny';

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

export function ChannelPermissionEditor({
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
