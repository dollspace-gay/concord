// ── Server types ────────────────────────────────────────

export interface UserProfile {
  id: string;
  username: string;
  email: string | null;
  avatar_url: string | null;
}

export interface AuthStatus {
  authenticated: boolean;
  providers: string[];
}

export interface ChannelInfo {
  name: string;
  topic: string;
  member_count: number;
}

export interface HistoryMessage {
  id: string;
  from: string;
  content: string;
  timestamp: string;
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
  | { type: 'message'; id: string; from: string; target: string; content: string; timestamp: string }
  | { type: 'join'; nickname: string; channel: string }
  | { type: 'part'; nickname: string; channel: string; reason?: string }
  | { type: 'quit'; nickname: string; reason?: string }
  | { type: 'topic_change'; channel: string; set_by: string; topic: string }
  | { type: 'nick_change'; old_nick: string; new_nick: string }
  | { type: 'names'; channel: string; members: string[] }
  | { type: 'topic'; channel: string; topic: string }
  | { type: 'channel_list'; channels: ChannelInfo[] }
  | { type: 'history'; channel: string; messages: HistoryMessage[]; has_more: boolean }
  | { type: 'server_notice'; message: string }
  | { type: 'error'; code: string; message: string };

// Client → Server commands
export type ClientCommand =
  | { type: 'send_message'; channel: string; content: string }
  | { type: 'join_channel'; channel: string }
  | { type: 'part_channel'; channel: string; reason?: string }
  | { type: 'set_topic'; channel: string; topic: string }
  | { type: 'fetch_history'; channel: string; before?: string; limit?: number }
  | { type: 'list_channels' }
  | { type: 'get_members'; channel: string };
