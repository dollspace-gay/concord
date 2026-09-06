import type { ChatState, ChatStoreContext } from './types';

export function createModerationActions({ get }: ChatStoreContext): Pick<ChatState, 'kickMember' | 'banMember' | 'unbanMember' | 'listBans' | 'timeoutMember' | 'setSlowMode' | 'setNsfw' | 'bulkDeleteMessages' | 'getAuditLog' | 'createAutomodRule' | 'updateAutomodRule' | 'deleteAutomodRule' | 'listAutomodRules'> {
  return {
    // ── Phase 6: Moderation ──
    kickMember: (serverId: string, userId: string, reason?: string) => {
      return get().runLifecycleCommand(
        { type: 'kick_member', server_id: serverId, user_id: userId, reason },
        `moderation:${serverId}:member:${userId}`,
      );
    },

    banMember: (serverId: string, userId: string, reason?: string, deleteMessageDays?: number) => {
      return get().runLifecycleCommand(
        { type: 'ban_member', server_id: serverId, user_id: userId, reason, delete_message_days: deleteMessageDays },
        `moderation:${serverId}:member:${userId}`,
      );
    },

    unbanMember: (serverId: string, userId: string) => {
      return get().runLifecycleCommand(
        { type: 'unban_member', server_id: serverId, user_id: userId },
        `moderation:${serverId}:member:${userId}`,
      );
    },

    listBans: (serverId: string) => {
      get().ws?.send({ type: 'list_bans', server_id: serverId });
    },

    timeoutMember: (serverId: string, userId: string, timeoutUntil?: string, reason?: string) => {
      return get().runLifecycleCommand(
        { type: 'timeout_member', server_id: serverId, user_id: userId, timeout_until: timeoutUntil, reason },
        `moderation:${serverId}:member:${userId}`,
      );
    },

    setSlowMode: (serverId: string, channel: string, seconds: number) => {
      return get().runLifecycleCommand(
        { type: 'set_slow_mode', server_id: serverId, channel, seconds },
        `moderation:${serverId}:channel:${channel}:slowmode`,
      );
    },

    setNsfw: (serverId: string, channel: string, isNsfw: boolean) => {
      return get().runLifecycleCommand(
        { type: 'set_nsfw', server_id: serverId, channel, is_nsfw: isNsfw },
        `moderation:${serverId}:channel:${channel}:nsfw`,
      );
    },

    bulkDeleteMessages: (serverId: string, channel: string, messageIds: string[]) => {
      return get().runLifecycleCommand(
        { type: 'bulk_delete_messages', server_id: serverId, channel, message_ids: messageIds },
        `moderation:${serverId}:channel:${channel}:bulk-delete`,
      );
    },

    getAuditLog: (serverId: string, actionType?: string, limit?: number, before?: string) => {
      get().ws?.send({ type: 'get_audit_log', server_id: serverId, action_type: actionType, limit, before });
    },

    createAutomodRule: (serverId: string, name: string, ruleType: string, config: string, actionType: string, timeoutSeconds?: number) => {
      return get().runLifecycleCommand(
        { type: 'create_automod_rule', server_id: serverId, name, rule_type: ruleType, config, action_type: actionType, timeout_duration_seconds: timeoutSeconds },
        `moderation:${serverId}:automod:create`,
      );
    },

    updateAutomodRule: (serverId: string, ruleId: string, name: string, enabled: boolean, config: string, actionType: string, timeoutSeconds?: number) => {
      return get().runLifecycleCommand(
        { type: 'update_automod_rule', server_id: serverId, rule_id: ruleId, name, enabled, config, action_type: actionType, timeout_duration_seconds: timeoutSeconds },
        `moderation:${serverId}:automod:${ruleId}`,
      );
    },

    deleteAutomodRule: (serverId: string, ruleId: string) => {
      return get().runLifecycleCommand(
        { type: 'delete_automod_rule', server_id: serverId, rule_id: ruleId },
        `moderation:${serverId}:automod:${ruleId}`,
      );
    },

    listAutomodRules: (serverId: string) => {
      get().ws?.send({ type: 'list_automod_rules', server_id: serverId });
    }
  };
}
