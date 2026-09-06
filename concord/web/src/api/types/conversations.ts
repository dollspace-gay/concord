

export interface SearchResultMessage {
  id: string;
  from: string;
  content: string;
  timestamp: string;
  channel_id: string;
  channel_name: string;
  edited_at?: string | null;
}

export interface PinnedMessageInfo {
  id: string;
  message_id: string;
  channel_id: string;
  pinned_by: string;
  pinned_at: string;
  from: string;
  content: string;
  timestamp: string;
}

export interface ThreadInfo {
  id: string;
  name: string;
  channel_type: string; // 'public_thread' | 'private_thread'
  parent_message_id?: string | null;
  archived: boolean;
  auto_archive_minutes: number;
  message_count: number;
  created_at: string;
  creator_user_id?: string | null;
  state_version?: number;
  tag_ids?: string[];
  tags_version?: number;
}

export interface ForumTagInfo {
  id: string;
  name: string;
  emoji?: string | null;
  moderated: boolean;
  position: number;
}

export interface BookmarkInfo {
  id: string;
  message_id: string;
  channel_id: string;
  from: string;
  content: string;
  timestamp: string;
  note?: string | null;
  created_at: string;
}
