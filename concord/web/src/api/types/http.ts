import type { HistoryMessage } from './messages';

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
