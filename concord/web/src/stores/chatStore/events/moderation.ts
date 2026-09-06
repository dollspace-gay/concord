import type { ChatEvent as ServerEvent } from '../../../api/generated/contract';
import { channelKey } from '../../../api/types';
import { redactDeletedReplyPreviews, removeDeletedReferences } from '../references';
import type { ChatStoreContext } from '../types';

export function handleModerationEvents({ set, get }: ChatStoreContext, event: Extract<ServerEvent, { type: 'member_kick' | 'member_ban' | 'member_unban' | 'member_timeout' | 'slow_mode_update' | 'nsfw_update' | 'bulk_message_delete' | 'audit_log_entries' | 'ban_list' | 'automod_rule_list' | 'automod_rule_update' | 'automod_rule_delete' }>) {
  switch (event.type) {
    // ── Phase 6: Moderation events ──
    case 'member_kick': {
      const e = event as Extract<ServerEvent, { type: 'member_kick' }>;
      const prefix = e.server_id + ':';
      const newMembers = { ...get().members };
      for (const key of Object.keys(newMembers)) {
        if (key.startsWith(prefix)) {
          newMembers[key] = newMembers[key].filter(m => m.user_id !== e.user_id);
        }
      }
      set({ members: newMembers });
      break;
    }
    case 'member_ban': {
      const e = event as Extract<ServerEvent, { type: 'member_ban' }>;
      const prefix = e.server_id + ':';
      const newMembers = { ...get().members };
      for (const key of Object.keys(newMembers)) {
        if (key.startsWith(prefix)) {
          newMembers[key] = newMembers[key].filter(m => m.user_id !== e.user_id);
        }
      }
      set({ members: newMembers });
      break;
    }
    case 'member_unban':
      // No UI action needed — the ban list will be refreshed if viewing it
      break;
    case 'member_timeout':
      // Could update member UI to show timeout badge — for now just acknowledge
      break;
    case 'slow_mode_update': {
      const e = event as Extract<ServerEvent, { type: 'slow_mode_update' }>;
      const channels = get().channels[e.server_id] ?? [];
      set({
        channels: {
          ...get().channels,
          [e.server_id]: channels.map(ch =>
            ch.name === e.channel ? { ...ch, slowmode_seconds: e.seconds } : ch
          ),
        },
      });
      break;
    }
    case 'nsfw_update': {
      const e = event as Extract<ServerEvent, { type: 'nsfw_update' }>;
      const channels = get().channels[e.server_id] ?? [];
      set({
        channels: {
          ...get().channels,
          [e.server_id]: channels.map(ch =>
            ch.name === e.channel ? { ...ch, is_nsfw: e.is_nsfw } : ch
          ),
        },
      });
      break;
    }
    case 'bulk_message_delete': {
      const e = event as Extract<ServerEvent, { type: 'bulk_message_delete' }>;
      const key = channelKey(e.server_id, e.channel);
      const deleteSet = new Set(e.message_ids);
      set((state) => ({
        messages: {
          ...state.messages,
          [key]: redactDeletedReplyPreviews(
            (state.messages[key] ?? []).filter((message) => !deleteSet.has(message.id)), deleteSet,
          ),
        },
        ...removeDeletedReferences(state, deleteSet),
        deletedMessageIds: {
          ...state.deletedMessageIds,
          ...Object.fromEntries(e.message_ids.map((id) => [id, true as const])),
        },
      }));
      break;
    }
    case 'audit_log_entries': {
      const e = event as Extract<ServerEvent, { type: 'audit_log_entries' }>;
      set({
        auditLog: {
          ...get().auditLog,
          [e.server_id]: e.entries,
        },
      });
      break;
    }
    case 'ban_list': {
      const e = event as Extract<ServerEvent, { type: 'ban_list' }>;
      set({
        bans: {
          ...get().bans,
          [e.server_id]: e.bans,
        },
      });
      break;
    }
    case 'automod_rule_list': {
      const e = event as Extract<ServerEvent, { type: 'automod_rule_list' }>;
      set({
        automodRules: {
          ...get().automodRules,
          [e.server_id]: e.rules,
        },
      });
      break;
    }
    case 'automod_rule_update': {
      const e = event as Extract<ServerEvent, { type: 'automod_rule_update' }>;
      const existing = get().automodRules[e.server_id] ?? [];
      const idx = existing.findIndex(r => r.id === e.rule.id);
      const updated = idx >= 0
        ? existing.map(r => r.id === e.rule.id ? e.rule : r)
        : [...existing, e.rule];
      set({
        automodRules: {
          ...get().automodRules,
          [e.server_id]: updated,
        },
      });
      break;
    }
    case 'automod_rule_delete': {
      const e = event as Extract<ServerEvent, { type: 'automod_rule_delete' }>;
      const existing = get().automodRules[e.server_id] ?? [];
      set({
        automodRules: {
          ...get().automodRules,
          [e.server_id]: existing.filter(r => r.id !== e.rule_id),
        },
      });
      break;
    }
  }
}
