use super::{Deserialize, JsonSchema, Serialize};

/// Audit log entry sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuditLogEntry {
    pub id: String,
    pub actor_id: String,
    pub actor_username_snapshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_avatar_snapshot: Option<String>,
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<String>,
    pub created_at: String,
}

/// Ban info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BanInfo {
    pub id: String,
    pub user_id: String,
    pub banned_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: String,
}

/// AutoMod rule info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutomodRuleInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub rule_type: String,
    pub config: String,
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_duration_seconds: Option<i32>,
}
