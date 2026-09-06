use super::{Deserialize, JsonSchema, Serialize};

/// Role metadata sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RoleInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub position: i32,
    pub permissions: i64,
    pub is_default: bool,
}

/// A role- or member-specific permission rule for one channel.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ChannelPermissionOverrideInfo {
    pub id: String,
    pub channel_id: String,
    pub target_type: String,
    pub target_id: String,
    pub allow_bits: i64,
    pub deny_bits: i64,
}

/// Channel category metadata sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CategoryInfo {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub position: i32,
}

/// Minimal channel position info for reorder events.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelPositionInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub position: i32,
}
