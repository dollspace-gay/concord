use serde::{Deserialize, Serialize};

/// A stored server (guild) from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub owner_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A server membership record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerMemberRow {
    pub server_id: String,
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}

/// A stored message from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MessageRow {
    pub id: String,
    pub server_id: Option<String>,
    pub channel_id: Option<String>,
    pub sender_id: String,
    pub sender_nick: String,
    pub content: String,
    pub created_at: String,
    pub target_user_id: Option<String>,
}

/// A stored channel from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub topic: String,
    pub topic_set_by: Option<String>,
    pub topic_set_at: Option<String>,
    pub created_at: String,
    pub is_default: i32,
}

/// A channel membership record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelMemberRow {
    pub channel_id: String,
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}
