import type { MessageComponent, RichEmbedInfo } from './rich_messages';

export interface ReplyInfo {
  id: string;
  from: string;
  content_preview: string;
}

export interface ReactionGroup {
  emoji: string;
  count: number;
  user_ids: string[];
}

export interface AttachmentInfo {
  id: string;
  filename: string;
  content_type: string;
  file_size: number;
  url: string;
}

export interface EmbedInfo {
  url: string;
  title?: string | null;
  description?: string | null;
  image_url?: string | null;
  site_name?: string | null;
}

export interface HistoryMessage {
  id: string;
  from: string;
  sender_id?: string;
  sequence?: string;
  deleted?: boolean;
  content: string;
  timestamp: string;
  edited_at?: string | null;
  reply_to?: ReplyInfo | null;
  reactions?: ReactionGroup[] | null;
  attachments?: AttachmentInfo[] | null;
  embeds?: EmbedInfo[] | null;
  rich_embeds?: RichEmbedInfo[] | null;
  components?: MessageComponent[] | null;
}

export interface UnreadCount {
  channel_name: string;
  count: number;
}
