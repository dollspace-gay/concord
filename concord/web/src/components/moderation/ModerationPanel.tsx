import { useState, useEffect } from 'react';
import { useChatStore } from '../../stores/chatStore';
import type { AuditLogEntry, BanInfo, AutomodRuleInfo, ChannelInfo } from '../../api/types';
import { Dialog } from '../Dialog';

interface Props {
  serverId: string;
  onClose: () => void;
}

type Tab = 'actions' | 'channels' | 'bans' | 'audit' | 'automod';
const TAB_LABELS: Record<Tab, string> = {
  actions: 'Member Actions',
  channels: 'Channel Safety',
  bans: 'Bans',
  audit: 'Audit Log',
  automod: 'AutoMod',
};
const EMPTY_BANS: BanInfo[] = [];
const EMPTY_AUDIT: AuditLogEntry[] = [];
const EMPTY_AUTOMOD: AutomodRuleInfo[] = [];
const EMPTY_CHANNELS: ChannelInfo[] = [];

function useActionStatus() {
  const [pending, setPending] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ kind: 'success' | 'error'; message: string } | null>(null);
  const run = async (key: string, action: () => Promise<void>, success: string, accepted?: () => void) => {
    if (pending) return;
    setPending(key);
    setOutcome(null);
    try {
      await action();
      accepted?.();
      setOutcome({ kind: 'success', message: success });
    } catch (cause) {
      setOutcome({ kind: 'error', message: cause instanceof Error ? cause.message : 'The action was rejected.' });
    } finally {
      setPending(null);
    }
  };
  return { pending, outcome, run };
}

function ActionOutcome({ outcome }: { outcome: { kind: 'success' | 'error'; message: string } | null }) {
  if (!outcome) return null;
  return <p role={outcome.kind === 'error' ? 'alert' : 'status'} className={`text-sm ${outcome.kind === 'error' ? 'text-red-400' : 'text-green-400'}`}>{outcome.message}</p>;
}

export function ModerationPanel({ serverId, onClose }: Props) {
  const [tab, setTab] = useState<Tab>('actions');
  const bans = useChatStore(s => s.bans[serverId] ?? EMPTY_BANS);
  const auditLog = useChatStore(s => s.auditLog[serverId] ?? EMPTY_AUDIT);
  const automodRules = useChatStore(s => s.automodRules[serverId] ?? EMPTY_AUTOMOD);
  const listBans = useChatStore(s => s.listBans);
  const getAuditLog = useChatStore(s => s.getAuditLog);
  const listAutomodRules = useChatStore(s => s.listAutomodRules);
  const unbanMember = useChatStore(s => s.unbanMember);
  const deleteAutomodRule = useChatStore(s => s.deleteAutomodRule);
  const createAutomodRule = useChatStore(s => s.createAutomodRule);
  const updateAutomodRule = useChatStore(s => s.updateAutomodRule);
  const kickMember = useChatStore(s => s.kickMember);
  const banMember = useChatStore(s => s.banMember);
  const timeoutMember = useChatStore(s => s.timeoutMember);
  const setSlowMode = useChatStore(s => s.setSlowMode);
  const setNsfw = useChatStore(s => s.setNsfw);
  const bulkDeleteMessages = useChatStore(s => s.bulkDeleteMessages);
  const channels = useChatStore(s => s.channels[serverId] ?? EMPTY_CHANNELS);

  useEffect(() => {
    listBans(serverId);
    getAuditLog(serverId);
    listAutomodRules(serverId);
  }, [serverId, listBans, getAuditLog, listAutomodRules]);

  useEffect(() => {
    if (tab === 'bans') listBans(serverId);
    if (tab === 'audit') getAuditLog(serverId);
    if (tab === 'automod') listAutomodRules(serverId);
  }, [tab, serverId, listBans, getAuditLog, listAutomodRules]);

  return (
    <Dialog label="Moderation" onClose={onClose} panelClassName="w-full max-w-2xl max-h-[80vh] flex flex-col rounded-lg bg-bg-primary shadow-xl">
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex items-center justify-between border-b border-border p-4">
          <h2 className="text-lg font-bold text-text-primary">Moderation</h2>
          <button onClick={onClose} aria-label="Close moderation" className="text-text-muted hover:text-text-primary">&times;</button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-border">
          {(['actions', 'channels', 'bans', 'audit', 'automod'] as Tab[]).map(t => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`px-4 py-2 text-sm font-medium capitalize ${
                tab === t ? 'border-b-2 border-bg-accent text-text-primary' : 'text-text-muted hover:text-text-secondary'
              }`}
            >
              {TAB_LABELS[t]}
            </button>
          ))}
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-4">
          {tab === 'bans' && (
            <BanListTab bans={bans} serverId={serverId} onUnban={unbanMember} />
          )}
          {tab === 'actions' && (
            <MemberActionsTab
              serverId={serverId}
              onKick={kickMember}
              onBan={banMember}
              onTimeout={timeoutMember}
            />
          )}
          {tab === 'channels' && (
            <ChannelModerationTab
              serverId={serverId}
              channels={channels}
              onSetSlowMode={setSlowMode}
              onSetNsfw={setNsfw}
              onBulkDelete={bulkDeleteMessages}
            />
          )}
          {tab === 'audit' && (
            <AuditLogTab entries={auditLog} />
          )}
          {tab === 'automod' && (
            <AutomodTab
              rules={automodRules}
              serverId={serverId}
              onCreate={createAutomodRule}
              onUpdate={updateAutomodRule}
              onDelete={deleteAutomodRule}
            />
          )}
        </div>
      </div>
    </Dialog>
  );
}

function MemberActionsTab({
  serverId,
  onKick,
  onBan,
  onTimeout,
}: {
  serverId: string;
  onKick: (serverId: string, userId: string, reason?: string) => Promise<void>;
  onBan: (serverId: string, userId: string, reason?: string, deleteMessageDays?: number) => Promise<void>;
  onTimeout: (serverId: string, userId: string, timeoutUntil?: string, reason?: string) => Promise<void>;
}) {
  const [userId, setUserId] = useState('');
  const [reason, setReason] = useState('');
  const [deleteDays, setDeleteDays] = useState(0);
  const [timeoutMinutes, setTimeoutMinutes] = useState(60);
  const { pending, outcome, run } = useActionStatus();
  const target = userId.trim();
  return (
    <div className="space-y-4">
      <div>
        <label htmlFor="moderation-user-id" className="mb-1 block text-sm font-medium text-text-secondary">User ID</label>
        <input
          id="moderation-user-id"
          value={userId}
          onChange={event => setUserId(event.target.value)}
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary outline-none"
          placeholder="User ID"
        />
      </div>
      <div>
        <label htmlFor="moderation-reason" className="mb-1 block text-sm font-medium text-text-secondary">Reason</label>
        <input
          id="moderation-reason"
          value={reason}
          maxLength={512}
          onChange={event => setReason(event.target.value)}
          className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary outline-none"
          placeholder="Optional reason"
        />
      </div>
      <div className="flex flex-wrap items-end gap-2">
        <button
          disabled={!target || pending !== null}
          onClick={() => run('kick', () => onKick(serverId, target, reason.trim() || undefined), 'Member kicked.')}
          className="rounded bg-amber-600 px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
        >{pending === 'kick' ? 'Kicking…' : 'Kick'}</button>
        <label className="text-xs text-text-secondary">
          Delete message days
          <select value={deleteDays} onChange={event => setDeleteDays(Number(event.target.value))} className="ml-2 rounded bg-bg-input px-2 py-2 text-text-primary">
            {[0, 1, 2, 3, 4, 5, 6, 7].map(days => <option key={days} value={days}>{days}</option>)}
          </select>
        </label>
        <button
          disabled={!target || pending !== null}
          onClick={() => run('ban', () => onBan(serverId, target, reason.trim() || undefined, deleteDays), 'Member banned.')}
          className="rounded bg-red-600 px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
        >{pending === 'ban' ? 'Banning…' : 'Ban'}</button>
        <label className="text-xs text-text-secondary">
          Timeout minutes
          <input type="number" min={1} max={40320} value={timeoutMinutes} onChange={event => setTimeoutMinutes(Number(event.target.value))} className="ml-2 w-24 rounded bg-bg-input px-2 py-2 text-text-primary" />
        </label>
        <button
          disabled={!target || timeoutMinutes < 1 || timeoutMinutes > 40320 || pending !== null}
          onClick={() => run('timeout', () => onTimeout(serverId, target, new Date(Date.now() + timeoutMinutes * 60_000).toISOString(), reason.trim() || undefined), 'Member timed out.')}
          className="rounded bg-bg-accent px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
        >{pending === 'timeout' ? 'Applying…' : 'Timeout'}</button>
        <button
          disabled={!target || pending !== null}
          onClick={() => run('clear', () => onTimeout(serverId, target, undefined, reason.trim() || undefined), 'Timeout cleared.')}
          className="rounded bg-bg-secondary px-3 py-2 text-sm text-text-secondary disabled:opacity-50"
        >{pending === 'clear' ? 'Clearing…' : 'Clear timeout'}</button>
      </div>
      <ActionOutcome outcome={outcome} />
    </div>
  );
}

function ChannelModerationTab({
  serverId,
  channels,
  onSetSlowMode,
  onSetNsfw,
  onBulkDelete,
}: {
  serverId: string;
  channels: ChannelInfo[];
  onSetSlowMode: (serverId: string, channel: string, seconds: number) => Promise<void>;
  onSetNsfw: (serverId: string, channel: string, isNsfw: boolean) => Promise<void>;
  onBulkDelete: (serverId: string, channel: string, messageIds: string[]) => Promise<void>;
}) {
  const [channel, setChannel] = useState(channels[0]?.name ?? '');
  const [slowMode, setSlowMode] = useState(0);
  const [messageIds, setMessageIds] = useState('');
  const { pending, outcome, run } = useActionStatus();
  const selected = channels.find(entry => entry.name === channel);
  const ids = messageIds.split(/[\s,]+/).map(id => id.trim()).filter(Boolean);
  return (
    <div className="space-y-4">
      <div>
        <label htmlFor="moderation-channel" className="mb-1 block text-sm font-medium text-text-secondary">Channel</label>
        <select id="moderation-channel" value={channel} onChange={event => setChannel(event.target.value)} className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary">
          {channels.map(entry => <option key={entry.id} value={entry.name}>{entry.name}</option>)}
        </select>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <label className="text-sm text-text-secondary">Slow mode seconds
          <input type="number" min={0} max={21600} value={slowMode} onChange={event => setSlowMode(Number(event.target.value))} className="ml-2 w-28 rounded bg-bg-input px-2 py-2 text-text-primary" />
        </label>
        <button disabled={!channel || slowMode < 0 || slowMode > 21600 || pending !== null} onClick={() => run('slow', () => onSetSlowMode(serverId, channel, slowMode), 'Slow mode updated.')} className="rounded bg-bg-accent px-3 py-2 text-sm text-white disabled:opacity-50">{pending === 'slow' ? 'Applying…' : 'Apply slow mode'}</button>
        <label className="flex items-center gap-2 text-sm text-text-secondary">
          <input type="checkbox" checked={selected?.is_nsfw ?? false} disabled={!channel || pending !== null} onChange={event => { const value = event.target.checked; void run('nsfw', () => onSetNsfw(serverId, channel, value), 'NSFW setting updated.'); }} />
          NSFW
        </label>
      </div>
      <div>
        <label htmlFor="moderation-message-ids" className="mb-1 block text-sm font-medium text-text-secondary">Message IDs to delete</label>
        <textarea id="moderation-message-ids" value={messageIds} onChange={event => setMessageIds(event.target.value)} rows={4} placeholder="One ID per line or comma-separated" className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary outline-none" />
        <button disabled={!channel || ids.length === 0 || ids.length > 100 || pending !== null} onClick={() => run('bulk-delete', () => onBulkDelete(serverId, channel, ids), 'Messages deleted.', () => setMessageIds(''))} className="mt-2 rounded bg-red-600 px-3 py-2 text-sm text-white disabled:opacity-50">{pending === 'bulk-delete' ? 'Deleting…' : `Delete ${ids.length || ''} messages`}</button>
      </div>
      <ActionOutcome outcome={outcome} />
    </div>
  );
}

function BanListTab({ bans, serverId, onUnban }: { bans: BanInfo[]; serverId: string; onUnban: (serverId: string, userId: string) => Promise<void> }) {
  const { pending, outcome, run } = useActionStatus();
  if (bans.length === 0) {
    return <div className="space-y-2"><p className="text-text-muted text-sm">No bans.</p><ActionOutcome outcome={outcome} /></div>;
  }
  return (
    <div className="space-y-2">
      {bans.map(ban => (
        <div key={ban.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
          <div>
            <span className="text-sm font-medium text-text-primary">User: {ban.user_id}</span>
            {ban.reason && <p className="text-xs text-text-muted">Reason: {ban.reason}</p>}
            <p className="text-xs text-text-muted">Banned by: {ban.banned_by} on {new Date(ban.created_at).toLocaleDateString()}</p>
          </div>
          <button
            disabled={pending !== null}
            onClick={() => run(`unban:${ban.user_id}`, () => onUnban(serverId, ban.user_id), 'Member unbanned.')}
            className="rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700"
          >
            {pending === `unban:${ban.user_id}` ? 'Unbanning…' : 'Unban'}
          </button>
        </div>
      ))}
      <ActionOutcome outcome={outcome} />
    </div>
  );
}

function AuditLogTab({ entries }: { entries: AuditLogEntry[] }) {
  if (entries.length === 0) {
    return <p className="text-text-muted text-sm">No audit log entries.</p>;
  }

  const actionLabels: Record<string, string> = {
    member_kick: 'Kicked',
    member_ban: 'Banned',
    member_unban: 'Unbanned',
    member_timeout: 'Timed out',
  };

  return (
    <div className="space-y-2">
      {entries.map(entry => (
        <div key={entry.id} className="rounded bg-bg-secondary p-3">
          <div className="flex items-center gap-2">
            <span className="rounded bg-bg-accent/20 px-2 py-0.5 text-xs font-medium text-bg-accent">
              {actionLabels[entry.action_type] ?? entry.action_type}
            </span>
            <span className="text-xs text-text-muted">
              by {entry.actor_id}
            </span>
            <span className="ml-auto text-xs text-text-muted">
              {new Date(entry.created_at).toLocaleString()}
            </span>
          </div>
          {entry.target_id && (
            <p className="mt-1 text-xs text-text-secondary">Target: {entry.target_id}</p>
          )}
          {entry.reason && (
            <p className="mt-1 text-xs text-text-muted">Reason: {entry.reason}</p>
          )}
        </div>
      ))}
    </div>
  );
}

function AutomodTab({ rules, serverId, onCreate, onUpdate, onDelete }: {
  rules: AutomodRuleInfo[];
  serverId: string;
  onCreate: (serverId: string, name: string, ruleType: string, config: string, actionType: string, timeoutSeconds?: number) => Promise<void>;
  onUpdate: (serverId: string, ruleId: string, name: string, enabled: boolean, config: string, actionType: string, timeoutSeconds?: number) => Promise<void>;
  onDelete: (serverId: string, ruleId: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [ruleType, setRuleType] = useState('keyword');
  const [config, setConfig] = useState('{"keywords":["blocked phrase"]}');
  const [actionType, setActionType] = useState('delete');
  const [timeoutSeconds, setTimeoutSeconds] = useState(300);
  const { pending, outcome, run } = useActionStatus();
  const create = () => {
    if (!name.trim() || !config.trim()) return;
    void run('create', () => onCreate(serverId, name.trim(), ruleType, config.trim(), actionType, actionType === 'timeout' ? timeoutSeconds : undefined), 'Rule created.', () => setName(''));
  };
  return (
    <div className="space-y-2">
      <div className="space-y-2 rounded bg-bg-secondary p-3">
        <input aria-label="AutoMod rule name" value={name} onChange={event => setName(event.target.value)} placeholder="Rule name" className="w-full rounded bg-bg-input px-3 py-2 text-sm text-text-primary" />
        <div className="flex flex-wrap gap-2">
          <select aria-label="AutoMod rule type" value={ruleType} onChange={event => setRuleType(event.target.value)} className="rounded bg-bg-input px-2 py-2 text-sm text-text-primary">
            <option value="keyword">Keyword</option><option value="mention_spam">Mention spam</option><option value="link_filter">Link filter</option>
          </select>
          <select aria-label="AutoMod action" value={actionType} onChange={event => setActionType(event.target.value)} className="rounded bg-bg-input px-2 py-2 text-sm text-text-primary">
            <option value="delete">Delete</option><option value="timeout">Timeout</option><option value="flag">Flag</option>
          </select>
          {actionType === 'timeout' && <input aria-label="AutoMod timeout seconds" type="number" min={1} max={2419200} value={timeoutSeconds} onChange={event => setTimeoutSeconds(Number(event.target.value))} className="w-32 rounded bg-bg-input px-2 py-2 text-sm text-text-primary" />}
        </div>
        <textarea aria-label="AutoMod JSON configuration" value={config} onChange={event => setConfig(event.target.value)} rows={3} className="w-full rounded bg-bg-input px-3 py-2 font-mono text-xs text-text-primary" />
        <button onClick={create} disabled={!name.trim() || !config.trim() || pending !== null} className="rounded bg-bg-accent px-3 py-2 text-sm text-white disabled:opacity-50">{pending === 'create' ? 'Creating…' : 'Create rule'}</button>
        <ActionOutcome outcome={outcome} />
      </div>
      {rules.length === 0 && <p className="text-text-muted text-sm">No automod rules configured.</p>}
      {rules.map(rule => (
        <div key={rule.id} className="flex items-center justify-between rounded bg-bg-secondary p-3">
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium text-text-primary">{rule.name}</span>
              <span className={`rounded px-1.5 py-0.5 text-xs ${rule.enabled ? 'bg-green-600/20 text-green-400' : 'bg-gray-600/20 text-gray-400'}`}>
                {rule.enabled ? 'Enabled' : 'Disabled'}
              </span>
            </div>
            <p className="text-xs text-text-muted">Type: {rule.rule_type} | Action: {rule.action_type}</p>
          </div>
          <div className="flex gap-2">
            <button disabled={pending !== null} onClick={() => run(`update:${rule.id}`, () => onUpdate(serverId, rule.id, rule.name, !rule.enabled, rule.config, rule.action_type, rule.timeout_duration_seconds ?? undefined), 'Rule updated.')} className="rounded bg-bg-accent px-3 py-1 text-xs font-medium text-white disabled:opacity-50">{pending === `update:${rule.id}` ? 'Updating…' : rule.enabled ? 'Disable' : 'Enable'}</button>
            <button disabled={pending !== null} onClick={() => run(`delete:${rule.id}`, () => onDelete(serverId, rule.id), 'Rule deleted.')} className="rounded bg-red-600 px-3 py-1 text-xs font-medium text-white hover:bg-red-700 disabled:opacity-50">{pending === `delete:${rule.id}` ? 'Deleting…' : 'Delete'}</button>
          </div>
        </div>
      ))}
    </div>
  );
}
