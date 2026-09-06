

// ── Phase 9: AT Protocol Deep Integration ─────────────

export interface BlueskyIdentityInfo {
  did: string;
  bsky_handle: string | null;
  display_name: string | null;
  description: string | null;
  avatar_url: string | null;
  banner_url: string | null;
  followers_count: number | null;
  follows_count: number | null;
  last_profile_sync: string | null;
}

export interface BlueskyShareResult {
  success?: boolean;
  publication_id: string;
  status: string;
  post_uri: string | null;
  cid: string | null;
}

export interface AtprotoPublicationStatus {
  id: string;
  source_message_id: string;
  source_version: number;
  channel_id: string;
  status: 'pending' | 'published' | 'update_pending' | 'delete_pending' | 'deleted' | 'failed' | 'cancelled';
  remote_uri: string | null;
  remote_cid: string | null;
  safe_error_code: string | null;
  updated_at: string;
  retryable: boolean;
  reauthentication_required: boolean;
}

export interface AtprotoChannelPublicationPolicy {
  channel_id: string;
  eligible: boolean;
  channel_enabled: boolean;
  user_granted: boolean;
}
