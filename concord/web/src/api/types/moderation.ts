

export interface AuditLogEntry {
  id: string;
  actor_id: string;
  actor_username_snapshot: string;
  actor_avatar_snapshot?: string | null;
  action_type: string;
  target_type?: string | null;
  target_id?: string | null;
  reason?: string | null;
  changes?: string | null;
  created_at: string;
}

export interface BanInfo {
  id: string;
  user_id: string;
  banned_by: string;
  reason?: string | null;
  created_at: string;
}

export interface AutomodRuleInfo {
  id: string;
  name: string;
  enabled: boolean;
  rule_type: string; // 'keyword' | 'mention_spam' | 'link_filter'
  config: string; // JSON string
  action_type: string; // 'delete' | 'timeout' | 'flag'
  timeout_duration_seconds?: number | null;
}
