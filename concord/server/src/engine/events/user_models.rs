use super::{Deserialize, JsonSchema, Serialize};

/// User presence info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PresenceInfo {
    pub user_id: String,
    pub nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
}

/// Full user profile info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserProfileInfo {
    pub user_id: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner_url: Option<String>,
    pub created_at: String,
}

/// Notification setting info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationSettingInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub level: String,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_until: Option<String>,
}

/// A search result message (same as HistoryMessage but with channel info).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchResultMessage {
    pub id: String,
    pub from: String,
    pub content: String,
    pub timestamp: String,
    pub channel_id: String,
    pub channel_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
}
