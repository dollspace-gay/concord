use super::{Deserialize, Serialize};

/// A server ban record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BanRow {
    pub id: String,
    pub server_id: String,
    pub user_id: String,
    pub banned_by: String,
    pub reason: Option<String>,
    pub delete_message_days: i32,
    pub created_at: String,
}

/// An audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogRow {
    pub id: String,
    pub server_id: String,
    pub actor_id: String,
    pub actor_username_snapshot: String,
    pub actor_avatar_snapshot: Option<String>,
    pub action_type: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub reason: Option<String>,
    pub changes: Option<String>,
    pub created_at: String,
}

/// An automod rule.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AutomodRuleRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub enabled: i32,
    pub rule_type: String,
    pub config: String,
    pub action_type: String,
    pub timeout_duration_seconds: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

/// Parameters for creating an audit log entry (avoids too-many-arguments).
pub struct CreateAuditLogParams<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub actor_id: &'a str,
    pub action_type: &'a str,
    pub target_type: Option<&'a str>,
    pub target_id: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub changes: Option<&'a str>,
}

/// Parameters for creating an automod rule (avoids too-many-arguments).
pub struct CreateAutomodRuleParams<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub rule_type: &'a str,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

/// Parameters for updating an automod rule (avoids too-many-arguments).
pub struct UpdateAutomodRuleParams<'a> {
    pub rule_id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}
