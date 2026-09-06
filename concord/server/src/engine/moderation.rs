use crate::auth::authority::AuthError;

use crate::auth::authority::{Actor, AuthService};

use crate::engine::authorization::AuthorizationError;

use crate::engine::authorization::AuthorizationService;

use crate::engine::ids::{ChannelId, ServerId};

use crate::engine::permissions::Permissions;

use crate::engine::write_admission::WriteAdmissionError;

use chrono::Utc;

use sqlx::{Row, SqliteConnection, SqlitePool};

use std::collections::HashSet;

use uuid::Uuid;

pub struct CreateAutomodRule<'a> {
    pub server_id: &'a ServerId,
    pub name: &'a str,
    pub rule_type: &'a str,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

pub struct UpdateAutomodRule<'a> {
    pub server_id: &'a ServerId,
    pub rule_id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModerationError {
    #[error("{0}")]
    Validation(String),
    #[error("moderation authentication failed")]
    Unauthenticated,
    #[error("moderation authorization failed")]
    Authorization(#[from] AuthorizationError),
    #[error("moderation write admission failed")]
    Admission(#[from] WriteAdmissionError),
    #[error("moderation database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("moderation dependency unavailable")]
    DependencyUnavailable,
    #[error("resource unavailable")]
    Unavailable,
}

impl ModerationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "INVALID_INPUT",
            Self::Unauthenticated
            | Self::Authorization(AuthorizationError::Authentication(
                AuthError::Invalid
                | AuthError::Revoked
                | AuthError::Expired
                | AuthError::Disabled
                | AuthError::Token(_),
            )) => "UNAUTHENTICATED",
            Self::Authorization(AuthorizationError::Authentication(
                AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_)
            | Self::DependencyUnavailable => "DEPENDENCY_UNAVAILABLE",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "RESOURCE_UNAVAILABLE"
            }
        }
    }

    pub fn safe_message(&self) -> &str {
        match self {
            Self::Validation(message) => message,
            Self::Unauthenticated
            | Self::Authorization(AuthorizationError::Authentication(
                AuthError::Invalid
                | AuthError::Revoked
                | AuthError::Expired
                | AuthError::Disabled
                | AuthError::Token(_),
            )) => "authentication required",
            Self::Authorization(AuthorizationError::Authentication(
                AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_)
            | Self::DependencyUnavailable => "dependency unavailable",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "resource unavailable"
            }
        }
    }

    pub fn wire_message(&self) -> String {
        format!("{}: {}", self.code(), self.safe_message())
    }
}

#[derive(Clone)]
pub struct ModerationService {
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

impl ModerationService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            auth,
            writes,
        }
    }
    async fn check_member_hierarchy_in(
        connection: &mut SqliteConnection,
        server_id: &str,
        actor_user_id: &str,
        target_user_id: &str,
    ) -> Result<(), ModerationError> {
        let owner_id: Option<String> = sqlx::query_scalar(
            "SELECT s.owner_id FROM servers s \
             JOIN server_members target ON target.server_id=s.id AND target.user_id=? \
             WHERE s.id=?",
        )
        .bind(target_user_id)
        .bind(server_id)
        .fetch_optional(&mut *connection)
        .await?;
        let owner_id = owner_id.ok_or(ModerationError::Unavailable)?;
        if target_user_id == owner_id {
            return Err(ModerationError::Unavailable);
        }
        if actor_user_id == owner_id {
            return Ok(());
        }
        let actor_position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(r.position),0) FROM user_roles ur \
             JOIN roles r ON r.id=ur.role_id AND r.server_id=ur.server_id \
             WHERE ur.server_id=? AND ur.user_id=?",
        )
        .bind(server_id)
        .bind(actor_user_id)
        .fetch_one(&mut *connection)
        .await?;
        let target_position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(r.position),0) FROM user_roles ur \
             JOIN roles r ON r.id=ur.role_id AND r.server_id=ur.server_id \
             WHERE ur.server_id=? AND ur.user_id=?",
        )
        .bind(server_id)
        .bind(target_user_id)
        .fetch_one(&mut *connection)
        .await?;
        if actor_position <= target_position {
            return Err(ModerationError::Unavailable);
        }
        Ok(())
    }
    fn validate_reason(reason: Option<&str>) -> Result<(), ModerationError> {
        if reason.is_some_and(|value| {
            value.len() > 512 || value.chars().any(|character| character == '\0')
        }) {
            return Err(ModerationError::Validation(
                "moderation reason must contain at most 512 bytes".into(),
            ));
        }
        Ok(())
    }
}

fn validate_automod_rule(
    name: &str,
    rule_type: &str,
    config: &str,
    action_type: &str,
    timeout_duration_seconds: Option<i32>,
) -> Result<(), ModerationError> {
    if name.trim().is_empty() || name.len() > 100 {
        return Err(ModerationError::Validation(
            "AutoMod rule name must contain 1 to 100 bytes".into(),
        ));
    }
    validate_automod_config(rule_type, config)?;
    validate_automod_action(action_type, timeout_duration_seconds)
}

fn validate_automod_config(rule_type: &str, config: &str) -> Result<(), ModerationError> {
    if config.len() > 64 * 1024 {
        return Err(ModerationError::Validation(
            "AutoMod config exceeds 65536 bytes".into(),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(config)
        .map_err(|_| ModerationError::Validation("Invalid JSON in automod config".into()))?;
    match rule_type {
        "keyword" => {
            let words = parsed
                .get("words")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    ModerationError::Validation("keyword config must have a 'words' array".into())
                })?;
            if words.is_empty() || words.len() > 1_000 {
                return Err(ModerationError::Validation(
                    "keyword config must contain 1 to 1000 entries".into(),
                ));
            }
            if words.iter().any(|word| {
                word.as_str()
                    .is_none_or(|word| word.trim().is_empty() || word.len() > 100)
            }) {
                return Err(ModerationError::Validation(
                    "keyword entries must contain 1 to 100 bytes".into(),
                ));
            }
        }
        "mention_spam" => {
            let maximum = parsed
                .get("max_mentions")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| {
                    ModerationError::Validation(
                        "mention_spam config must have a 'max_mentions' integer".into(),
                    )
                })?;
            if !(1..=100).contains(&maximum) {
                return Err(ModerationError::Validation(
                    "mention_spam 'max_mentions' must be between 1 and 100".into(),
                ));
            }
        }
        "link_filter" => {
            let block_all = parsed.get("block_all");
            let domains = parsed.get("allowed_domains");
            if block_all.is_none() && domains.is_none() {
                return Err(ModerationError::Validation(
                    "link_filter config must have 'block_all' or 'allowed_domains'".into(),
                ));
            }
            if block_all.is_some_and(|value| !value.is_boolean()) {
                return Err(ModerationError::Validation(
                    "link_filter 'block_all' must be a boolean".into(),
                ));
            }
            if let Some(domains) = domains {
                let domains = domains.as_array().ok_or_else(|| {
                    ModerationError::Validation(
                        "link_filter 'allowed_domains' must be an array".into(),
                    )
                })?;
                if domains.len() > 1_000
                    || domains.iter().any(|domain| {
                        domain.as_str().is_none_or(|domain| {
                            domain.is_empty()
                                || domain.len() > 253
                                || domain.contains('/')
                                || domain.contains(':')
                        })
                    })
                {
                    return Err(ModerationError::Validation(
                        "link_filter contains an invalid domain".into(),
                    ));
                }
            }
        }
        _ => {
            return Err(ModerationError::Validation(
                "Invalid rule type. Must be 'keyword', 'mention_spam', or 'link_filter'".into(),
            ));
        }
    }
    Ok(())
}

fn validate_automod_action(
    action_type: &str,
    timeout_duration_seconds: Option<i32>,
) -> Result<(), ModerationError> {
    match (action_type, timeout_duration_seconds) {
        ("timeout", Some(seconds)) if (1..=2_419_200).contains(&seconds) => Ok(()),
        ("timeout", _) => Err(ModerationError::Validation(
            "timeout action requires a duration between 1 and 2419200 seconds".into(),
        )),
        ("delete" | "flag", None) => Ok(()),
        ("delete" | "flag", Some(_)) => Err(ModerationError::Validation(
            "only timeout actions may specify timeout_duration_seconds".into(),
        )),
        _ => Err(ModerationError::Validation(
            "Invalid action type. Must be 'delete', 'timeout', or 'flag'".into(),
        )),
    }
}

mod automod;
mod channels;
mod membership;
