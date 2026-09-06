use super::{Deserialize, Serialize};

/// User presence and custom status.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserPresenceRow {
    pub user_id: String,
    pub status: String,
    pub requested_status: String,
    pub custom_status: Option<String>,
    pub status_emoji: Option<String>,
    pub last_seen_at: String,
    pub updated_at: String,
}

/// User profile (bio, pronouns, banner).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserProfileRow {
    pub user_id: String,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub banner_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Parameters for upserting a notification setting (avoids too-many-arguments).
pub struct UpsertNotificationParams<'a> {
    pub id: &'a str,
    pub user_id: &'a str,
    pub server_id: Option<&'a str>,
    pub channel_id: Option<&'a str>,
    pub level: &'a str,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    pub mute_until: Option<&'a str>,
}

/// Per-server/channel notification settings.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NotificationSettingRow {
    pub id: String,
    pub user_id: String,
    pub server_id: Option<String>,
    pub channel_id: Option<String>,
    pub level: String,
    pub suppress_everyone: i32,
    pub suppress_roles: i32,
    pub muted: i32,
    pub mute_until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
