use std::collections::HashSet;

use sqlx::{Row, SqlitePool};

use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService};

use crate::db::models::{CreateAuditLogParams, ForumTagRow};

use crate::engine::authorization::{AuthorizationError, AuthorizationService, ChannelAction};

use crate::engine::events::ForumTagInfo;

use crate::engine::permissions::Permissions;

use crate::engine::write_admission::{WriteAdmission, WriteAdmissionError};

#[derive(Debug, thiserror::Error)]
pub enum ForumError {
    #[error("{0}")]
    Validation(&'static str),
    #[error("forum authorization failed")]
    Authorization(#[from] AuthorizationError),
    #[error("forum write admission failed")]
    Admission(#[from] WriteAdmissionError),
    #[error("forum database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("resource unavailable")]
    Unavailable,
    #[error("thread tag version changed concurrently")]
    Conflict,
}

impl ForumError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "INVALID_INPUT",
            Self::Conflict => "CONFLICT",
            Self::Authorization(AuthorizationError::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_) => "DEPENDENCY_UNAVAILABLE",
            Self::Authorization(AuthorizationError::Authentication(_)) => "UNAUTHENTICATED",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "RESOURCE_UNAVAILABLE"
            }
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Validation(message) => message,
            Self::Conflict => "thread tag version changed concurrently",
            Self::Authorization(AuthorizationError::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_) => "dependency unavailable",
            Self::Authorization(AuthorizationError::Authentication(_)) => "authentication required",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "resource unavailable"
            }
        }
    }

    pub fn wire_message(&self) -> String {
        format!("{}: {}", self.code(), self.safe_message())
    }
}

pub struct CreateForumTag<'a> {
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub name: &'a str,
    pub emoji: Option<&'a str>,
    pub moderated: bool,
}

pub struct UpdateForumTag<'a> {
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub tag_id: &'a str,
    pub name: &'a str,
    pub emoji: Option<&'a str>,
    pub moderated: bool,
    pub position: i32,
}

pub struct ThreadTagMutation {
    pub thread_id: String,
    pub version: i64,
    pub tag_ids: Vec<String>,
}

#[derive(Clone)]
pub struct ForumService {
    pool: SqlitePool,
    writes: WriteAdmission,
}

impl ForumService {
    pub fn new(pool: SqlitePool, writes: WriteAdmission) -> Self {
        Self { pool, writes }
    }
}

async fn require_forum_channel(
    connection: &mut sqlx::SqliteConnection,
    server_id: &str,
    channel_id: &str,
) -> Result<(), ForumError> {
    let is_forum: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM channels \
         WHERE id=? AND server_id=? AND channel_type='forum')",
    )
    .bind(channel_id)
    .bind(server_id)
    .fetch_one(connection)
    .await?;
    if is_forum {
        Ok(())
    } else {
        Err(ForumError::Validation("forum tags require a forum channel"))
    }
}

async fn insert_audit(
    connection: &mut sqlx::SqliteConnection,
    server_id: &str,
    actor: &Actor,
    action_type: &str,
    target_type: &str,
    target_id: &str,
    changes: Option<&str>,
) -> Result<(), ForumError> {
    let id = Uuid::new_v4().to_string();
    crate::db::queries::audit_log::create_entry_in(
        connection,
        &CreateAuditLogParams {
            id: &id,
            server_id,
            actor_id: actor.user_id().as_str(),
            action_type,
            target_type: Some(target_type),
            target_id: Some(target_id),
            reason: None,
            changes,
        },
    )
    .await?;
    Ok(())
}

fn validate_tag(name: &str, emoji: Option<&str>, position: Option<i32>) -> Result<(), ForumError> {
    if name.trim().is_empty() || name.len() > 100 {
        return Err(ForumError::Validation(
            "forum tag name must contain 1 to 100 bytes",
        ));
    }
    if emoji.is_some_and(|value| value.is_empty() || value.len() > 100) {
        return Err(ForumError::Validation(
            "forum tag emoji must contain 1 to 100 bytes",
        ));
    }
    if position.is_some_and(|value| !(0..20).contains(&value)) {
        return Err(ForumError::Validation(
            "forum tag position must be between 0 and 19",
        ));
    }
    Ok(())
}

fn row_to_info(row: ForumTagRow) -> ForumTagInfo {
    ForumTagInfo {
        id: row.id,
        name: row.name,
        emoji: row.emoji,
        moderated: row.moderated != 0,
        position: row.position,
    }
}

mod tags;
mod thread_tags;
