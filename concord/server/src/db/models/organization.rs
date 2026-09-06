use super::{Deserialize, Serialize};

/// A stored server (guild) from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub owner_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub description: Option<String>,
    pub is_discoverable: i32,
    pub welcome_message: Option<String>,
    pub rules_text: Option<String>,
    pub category: Option<String>,
    pub allow_external_emoji: i32,
    pub shareable_emoji: i32,
    pub vanity_code: Option<String>,
    pub authorization_version: i64,
}

/// A server membership record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerMemberRow {
    pub server_id: String,
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
    pub avatar_url: Option<String>,
}

/// A custom sticker in a server.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StickerRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub image_url: String,
    pub description: Option<String>,
    pub uploader_id: String,
    pub created_at: String,
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
    pub edited_at: Option<String>,
    pub deleted_at: Option<String>,
    pub reply_to_id: Option<String>,
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
    pub category_id: Option<String>,
    pub position: i32,
    pub is_private: i32,
    pub channel_type: String,
    pub thread_parent_message_id: Option<String>,
    pub thread_auto_archive_minutes: i32,
    pub thread_last_activity_at: Option<String>,
    pub thread_archive_due_at: Option<String>,
    pub thread_archive_reason: Option<String>,
    pub thread_state_version: i64,
    pub archived: i32,
    pub slowmode_seconds: i32,
    pub is_nsfw: i32,
    pub is_announcement: i32,
    pub authorization_version: i64,
    pub parent_channel_id: Option<String>,
    pub visibility_repair_required: i32,
    pub thread_creator_user_id: Option<String>,
    pub thread_tags_version: i64,
}

/// A channel membership record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelMemberRow {
    pub channel_id: String,
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}

/// A custom role within a server.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RoleRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub color: Option<String>,
    pub icon_url: Option<String>,
    pub position: i32,
    pub permissions: i64,
    pub is_default: i32,
    pub created_at: String,
}

/// A user-to-role assignment.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserRoleRow {
    pub server_id: String,
    pub user_id: String,
    pub role_id: String,
    pub assigned_at: String,
}

/// A channel category (grouping of channels).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelCategoryRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub position: i32,
    pub created_at: String,
}

/// A channel permission override.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelPermissionOverrideRow {
    pub id: String,
    pub channel_id: String,
    pub target_type: String,
    pub target_id: String,
    pub allow_bits: i64,
    pub deny_bits: i64,
    pub created_at: String,
}
