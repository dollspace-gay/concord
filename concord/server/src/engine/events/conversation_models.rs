use super::{Deserialize, JsonSchema, Serialize};

/// Info about a pinned message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PinnedMessageInfo {
    pub id: String,
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: String,
    // Denormalized message content for display
    pub from: String,
    pub content: String,
    pub timestamp: String,
}

/// Info about a thread.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThreadInfo {
    pub id: String,
    pub name: String,
    pub channel_type: String, // "public_thread" | "private_thread"
    pub parent_message_id: Option<String>,
    pub creator_user_id: Option<String>,
    pub archived: bool,
    #[serde(default)]
    pub state_version: i64,
    #[serde(default)]
    pub tags_version: i64,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    pub auto_archive_minutes: i32,
    pub message_count: i64,
    pub created_at: String,
}

/// Forum tag info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ForumTagInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub moderated: bool,
    pub position: i32,
}

/// Bookmark info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BookmarkInfo {
    pub id: String,
    pub message_id: String,
    pub channel_id: String,
    pub from: String,
    pub content: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: String,
}

/// Direct conversation navigation entry sent only to a participant.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DirectConversationInfo {
    pub id: String,
    pub peer_id: String,
    pub peer_username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    pub unread_count: u64,
}
