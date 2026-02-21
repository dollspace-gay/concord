use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::events::SessionId;

/// In-memory state for a single channel.
#[derive(Debug)]
pub struct ChannelState {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub topic: String,
    pub topic_set_by: Option<String>,
    pub topic_set_at: Option<DateTime<Utc>>,
    /// Session IDs of currently connected members.
    pub members: HashSet<SessionId>,
    pub created_at: DateTime<Utc>,
    /// Category this channel belongs to (None = uncategorized).
    pub category_id: Option<String>,
    /// Sort position within its category.
    pub position: i32,
    /// Whether this channel is private (members-only).
    pub is_private: bool,
    /// Channel type: "text", "public_thread", "private_thread", "forum".
    pub channel_type: String,
    /// For threads: the message ID this thread was created from.
    pub thread_parent_message_id: Option<String>,
    /// Auto-archive duration in minutes (default 1440 = 24h).
    pub auto_archive_minutes: i32,
    /// Whether this channel/thread is archived.
    pub archived: bool,
    /// Slow mode cooldown in seconds (0 = disabled).
    pub slowmode_seconds: i32,
    /// Whether this channel is marked NSFW.
    pub is_nsfw: bool,
}

impl ChannelState {
    pub fn new(id: String, server_id: String, name: String) -> Self {
        Self {
            id,
            server_id,
            name,
            topic: String::new(),
            topic_set_by: None,
            topic_set_at: None,
            members: HashSet::new(),
            created_at: Utc::now(),
            category_id: None,
            position: 0,
            is_private: false,
            channel_type: "text".to_string(),
            thread_parent_message_id: None,
            auto_archive_minutes: 1440,
            archived: false,
            slowmode_seconds: 0,
            is_nsfw: false,
        }
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }
}
