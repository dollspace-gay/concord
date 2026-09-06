

export interface InviteInfo {
  id: string;
  code: string;
  server_id: string;
  created_by: string;
  max_uses?: number | null;
  use_count: number;
  expires_at?: string | null;
  channel_id?: string | null;
  created_at: string;
}

export interface EventInfo {
  id: string;
  server_id: string;
  name: string;
  description?: string | null;
  channel_id?: string | null;
  start_time: string;
  end_time?: string | null;
  image_url?: string | null;
  created_by: string;
  status: string; // 'scheduled' | 'active' | 'completed' | 'cancelled'
  interested_count: number;
  created_at: string;
}

export interface RsvpInfo {
  user_id: string;
  status: string; // 'interested' | 'going'
}

export interface ChannelFollowInfo {
  id: string;
  source_channel_id: string;
  target_channel_id: string;
  created_by: string;
}

export interface TemplateInfo {
  id: string;
  name: string;
  description?: string | null;
  server_id: string;
  created_by: string;
  use_count: number;
  created_at: string;
}

export interface ServerCommunityInfo {
  server_id: string;
  description?: string | null;
  is_discoverable: boolean;
  welcome_message?: string | null;
  rules_text?: string | null;
  category?: string | null;
  rules_accepted?: boolean | null;
}
