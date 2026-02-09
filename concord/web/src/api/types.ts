// ── Server types ────────────────────────────────────────

export interface UserProfile {
  id: string;
  username: string;
  email: string | null;
  avatar_url: string | null;
  is_system_admin?: boolean;
}

export interface AuthStatus {
  authenticated: boolean;
  providers: string[];
}

export interface ServerInfo {
  id: string;
  name: string;
  icon_url?: string | null;
  member_count: number;
  role?: string | null;
}

export interface ChannelInfo {
  id: string;
  server_id: string;
  name: string;
  topic: string;
  member_count: number;
}

export interface MemberInfo {
  nickname: string;
  avatar_url?: string | null;
}

export interface HistoryMessage {
  id: string;
  from: string;
  content: string;
  timestamp: string;
}

export interface PublicUserProfile {
  username: string;
  avatar_url: string | null;
  provider: string | null;
  provider_id: string | null;
}

export interface HistoryResponse {
  channel: string;
  messages: HistoryMessage[];
  has_more: boolean;
}

export interface IrcToken {
  id: string;
  label: string | null;
  last_used: string | null;
  created_at: string;
}

export interface CreateTokenResponse {
  id: string;
  token: string;
  label: string | null;
}

// ── WebSocket message types ─────────────────────────────

// Server → Client events
export type ServerEvent =
  | { type: 'message'; id: string; server_id?: string; from: string; target: string; content: string; timestamp: string; avatar_url?: string }
  | { type: 'join'; nickname: string; server_id: string; channel: string; avatar_url?: string }
  | { type: 'part'; nickname: string; server_id: string; channel: string; reason?: string }
  | { type: 'quit'; nickname: string; reason?: string }
  | { type: 'topic_change'; server_id: string; channel: string; set_by: string; topic: string }
  | { type: 'nick_change'; old_nick: string; new_nick: string }
  | { type: 'names'; server_id: string; channel: string; members: MemberInfo[] }
  | { type: 'topic'; server_id: string; channel: string; topic: string }
  | { type: 'channel_list'; server_id: string; channels: ChannelInfo[] }
  | { type: 'history'; server_id: string; channel: string; messages: HistoryMessage[]; has_more: boolean }
  | { type: 'server_list'; servers: ServerInfo[] }
  | { type: 'server_notice'; message: string }
  | { type: 'error'; code: string; message: string };

// Client → Server commands
export type ClientCommand =
  | { type: 'send_message'; server_id: string; channel: string; content: string }
  | { type: 'join_channel'; server_id: string; channel: string }
  | { type: 'part_channel'; server_id: string; channel: string; reason?: string }
  | { type: 'set_topic'; server_id: string; channel: string; topic: string }
  | { type: 'fetch_history'; server_id: string; channel: string; before?: string; limit?: number }
  | { type: 'list_channels'; server_id: string }
  | { type: 'get_members'; server_id: string; channel: string }
  | { type: 'list_servers' }
  | { type: 'create_server'; name: string; icon_url?: string }
  | { type: 'join_server'; server_id: string }
  | { type: 'leave_server'; server_id: string }
  | { type: 'create_channel'; server_id: string; name: string }
  | { type: 'delete_channel'; server_id: string; channel: string }
  | { type: 'delete_server'; server_id: string }
  | { type: 'update_member_role'; server_id: string; user_id: string; role: string };

// ── Helpers ─────────────────────────────────────────────

/** Composite key for channel-scoped data: "server_id:channel_name" */
export function channelKey(serverId: string, channel: string): string {
  return `${serverId}:${channel}`;
}
