

export interface ServerInfo {
  id: string;
  name: string;
  icon_url?: string | null;
  member_count: number;
  role?: string | null;
  my_permissions?: number;
}

export interface ChannelInfo {
  id: string;
  conversation_id: string;
  server_id: string;
  name: string;
  topic: string;
  member_count: number;
  category_id?: string | null;
  position: number;
  is_private: boolean;
  channel_type: string;
  thread_parent_message_id?: string | null;
  archived: boolean;
  slowmode_seconds: number;
  is_nsfw: boolean;
}

export interface MemberInfo {
  nickname: string;
  avatar_url?: string | null;
  status?: string | null;
  custom_status?: string | null;
  status_emoji?: string | null;
  user_id?: string | null;
  server_avatar_url?: string | null;
  role_ids?: string[];
}

export interface StickerInfo {
  id: string;
  server_id: string;
  name: string;
  image_url: string;
  description?: string | null;
}
