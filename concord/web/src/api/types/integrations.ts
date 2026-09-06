

// ── Phase 8: Integrations & Bots ──────────────────────

export interface WebhookInfo {
  id: string;
  server_id: string;
  channel_id: string;
  name: string;
  avatar_url?: string | null;
  webhook_type: string; // 'incoming' | 'outgoing'
  token: string;
  url?: string | null;
  created_by: string;
  created_at: string;
}

export interface WebhookDeliveryStatus {
  delivery_id: string;
  event_type: string;
  event_version: number;
  state: 'pending' | 'leased' | 'delivered' | 'failed' | 'cancelled';
  attempt_count: number;
  last_status?: number | null;
  safe_error_code?: string | null;
  created_at: string;
  delivered_at?: string | null;
}

export interface SlashCommandInfo {
  id: string;
  bot_user_id: string;
  server_id?: string | null;
  name: string;
  description: string;
  options: SlashCommandOption[];
  created_at: string;
}

export interface SlashCommandOption {
  name: string;
  description: string;
  option_type: string; // 'string' | 'integer' | 'boolean' | 'user' | 'channel' | 'role'
  required: boolean;
  choices?: SlashCommandChoice[];
}

export interface SlashCommandChoice {
  name: string;
  value: string;
}

export interface InteractionInfo {
  id: string;
  interaction_type: string;
  command_id?: string | null;
  command_name?: string | null;
  user_id: string;
  server_id: string;
  channel_id: string;
  data_json: string;
}

export interface BotTokenInfo {
  id: string;
  name: string;
  scopes: string;
  created_at: string;
  last_used?: string | null;
}

export interface BotAccountInfo {
  id: string;
  username: string;
  avatar_url?: string | null;
  installed_server_ids: string[];
}

export interface OAuth2AppInfo {
  id: string;
  name: string;
  description: string;
  icon_url?: string | null;
  owner_id: string;
  redirect_uris: string;
  scopes: string;
  is_public: boolean;
  created_at: string;
}
