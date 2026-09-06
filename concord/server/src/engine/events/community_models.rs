use super::{Deserialize, JsonSchema, Serialize};

/// Server invite info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InviteInfo {
    pub id: String,
    pub code: String,
    pub server_id: String,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<i32>,
    pub use_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub created_at: String,
}

/// Scheduled event info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub start_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub created_by: String,
    pub status: String,
    pub interested_count: i64,
    pub created_at: String,
}

/// RSVP info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RsvpInfo {
    pub user_id: String,
    pub status: String,
}

/// Channel follow info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelFollowInfo {
    pub id: String,
    pub source_channel_id: String,
    pub target_channel_id: String,
    pub created_by: String,
}

/// Server template info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub server_id: String,
    pub created_by: String,
    pub use_count: i32,
    pub created_at: String,
}

/// Server community/discovery info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerCommunityInfo {
    pub server_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_discoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Whether the requesting member accepted the server's current rules version.
    /// Omitted from public discovery results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules_accepted: Option<bool>,
}
