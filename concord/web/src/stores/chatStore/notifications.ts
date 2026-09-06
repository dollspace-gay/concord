import type { NotificationSettingInfo } from '../../api/types';
import type { ChatState } from './types';

export function notificationPolicy(settings: NotificationSettingInfo[], channelId?: string) {
  const global = settings.find((setting) => !setting.server_id && !setting.channel_id);
  const server = settings.find((setting) => setting.server_id && !setting.channel_id);
  const channel = channelId ? settings.find((setting) => setting.channel_id === channelId) : undefined;
  const levels = [channel?.level, server?.level, global?.level];
  const level = levels.find((candidate) => candidate && candidate !== 'default') ?? 'mentions';
  const muteRows = [global, server, channel].filter((row): row is NotificationSettingInfo => Boolean(row));
  const muted = muteRows.some((row) => row.muted && (!row.mute_until || Date.parse(row.mute_until) > Date.now()));
  const controls = channel ?? server ?? global;
  return {
    level,
    muted,
    suppressEveryone: controls?.suppress_everyone ?? false,
    suppressRoles: controls?.suppress_roles ?? false,
  };
}

export async function claimDesktopNotification(accountId: string, messageId: string): Promise<boolean> {
  if (typeof navigator === 'undefined' || !navigator.locks) return false;
  const ledgerKey = `concord:notification-ledger:${accountId}`;
  return navigator.locks.request(`concord:notification-lock:${accountId}:${messageId}`, async () => {
    try {
      const now = Date.now();
      const parsed = JSON.parse(localStorage.getItem(ledgerKey) ?? '{}') as unknown;
      const ledger: Record<string, number> = parsed && typeof parsed === 'object'
        ? Object.fromEntries(Object.entries(parsed).filter((entry): entry is [string, number] =>
          typeof entry[1] === 'number' && Number.isFinite(entry[1]) && now - entry[1] < 86_400_000))
        : {};
      if (ledger[messageId] !== undefined) return false;
      ledger[messageId] = now;
      const bounded = Object.fromEntries(Object.entries(ledger)
        .sort((left, right) => right[1] - left[1])
        .slice(0, 512));
      localStorage.setItem(ledgerKey, JSON.stringify(bounded));
      return true;
    } catch {
      // Without an atomic durable claim, suppress the alert instead of risking
      // duplicate notifications across tabs.
      return false;
    }
  });
}

export async function maybeNotifyMessage(
  get: () => ChatState,
  state: ChatState,
  message: {
    id: string;
    senderId?: string;
    senderNick: string;
    content?: string | null;
    mentions?: Array<{ kind: string; target_id?: string | null }>;
  },
  serverId?: string,
  channelId?: string,
): Promise<void> {
  if (typeof document === 'undefined' || document.visibilityState === 'visible') return;
  if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
  if (!state.activeAccountId || message.senderId === state.activeAccountId || message.senderNick === state.nickname) return;
  const isDnd = state.ownPresenceStatus === 'dnd'
    || Object.values(state.presences).some((server) => server[state.activeAccountId!]?.status === 'dnd');
  if (isDnd) return;
  const settings = serverId
    ? state.notificationSettings[serverId] ?? []
    : Object.values(state.notificationSettings).flat();
  const policy = notificationPolicy(settings, channelId);
  if (policy.muted || policy.level === 'none') return;
  const mentions = message.mentions ?? [];
  const mentionsMe = mentions.some((mention) => mention.kind === 'user' && mention.target_id === state.activeAccountId)
    || (!policy.suppressEveryone && mentions.some((mention) => mention.kind === 'everyone'))
    || (!policy.suppressRoles && serverId !== undefined && mentions.some((mention) =>
      mention.kind === 'role'
      && mention.target_id !== undefined
      && mention.target_id !== null
      && (state.memberRoles[serverId]?.[state.activeAccountId!] ?? []).includes(mention.target_id)));
  if (policy.level === 'mentions' && !mentionsMe) return;
  const accountId = state.activeAccountId;
  const protectedGeneration = state.protectedGeneration;
  if (!await claimDesktopNotification(accountId, message.id)) return;
  const current = get();
  if (current.activeAccountId !== accountId || current.protectedGeneration !== protectedGeneration) return;
  new Notification(message.senderNick, {
    body: message.content?.trim() || 'Sent an attachment',
    tag: `concord-message-${message.id}`,
  });
}
