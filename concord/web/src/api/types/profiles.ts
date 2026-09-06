

export interface PresenceInfo {
  user_id: string;
  nickname: string;
  avatar_url?: string | null;
  status: string; // 'online' | 'idle' | 'dnd' | 'offline'
  custom_status?: string | null;
  status_emoji?: string | null;
}

export interface UserProfileInfo {
  user_id: string;
  username: string;
  avatar_url?: string | null;
  bio?: string | null;
  pronouns?: string | null;
  banner_url?: string | null;
  created_at: string;
}

export interface NotificationSettingInfo {
  id: string;
  server_id?: string | null;
  channel_id?: string | null;
  level: string; // 'all' | 'mentions' | 'none' | 'default'
  suppress_everyone: boolean;
  suppress_roles: boolean;
  muted: boolean;
  mute_until?: string | null;
}
