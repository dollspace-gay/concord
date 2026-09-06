use super::{Deserialize, Serialize};

/// A pinned message in a channel.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PinnedMessageRow {
    pub id: String,
    pub channel_id: String,
    pub message_id: String,
    pub pinned_by: String,
    pub pinned_at: String,
}

/// A forum tag for categorizing threads.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ForumTagRow {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub emoji: Option<String>,
    pub moderated: i32,
    pub position: i32,
    pub created_at: String,
}

/// A thread-to-tag association.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ThreadTagRow {
    pub thread_id: String,
    pub tag_id: String,
}

/// A personal bookmark on a message.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BookmarkRow {
    pub id: String,
    pub user_id: String,
    pub message_id: String,
    pub note: Option<String>,
    pub created_at: String,
}
