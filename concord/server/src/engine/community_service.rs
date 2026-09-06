use sqlx::{Row, SqliteConnection, SqlitePool};

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService};

use crate::db::models::{
    ChannelFollowRow, EventRsvpRow, InviteRow, ServerEventRow, ServerRow, ServerTemplateRow,
};

use super::authorization::{AuthorizationService, AuthorizationStamp, ChannelAction};

use super::ids::{ChannelId, ResourceIdError, ServerId};

use super::permissions::Permissions;

#[derive(Debug)]
pub enum CommunityError {
    Authentication(crate::auth::authority::AuthError),
    Forbidden,
    InvalidInput(&'static str),
    Conflict(&'static str),
    Admission(super::write_admission::WriteAdmissionError),
    Database(sqlx::Error),
    InvalidStoredResourceId(ResourceIdError),
}

pub struct PublicInvitePreview {
    pub code: String,
    pub server_id: String,
    pub server_name: String,
    pub server_icon_url: Option<String>,
    pub is_vanity: bool,
}

pub enum PublicInvitePreviewError {
    ExpiredOrExhausted,
    DependencyUnavailable,
    Database(sqlx::Error),
}

#[derive(sqlx::FromRow)]
struct PublicInviteRow {
    code: String,
    server_id: String,
    expires_at: Option<String>,
    max_uses: Option<i32>,
    use_count: i32,
    server_name: String,
    server_icon_url: Option<String>,
}

impl CommunityError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            )
            | Self::Admission(_)
            | Self::Database(_)
            | Self::InvalidStoredResourceId(_) => "DEPENDENCY_UNAVAILABLE",
            Self::Authentication(_) => "UNAUTHENTICATED",
            Self::Forbidden => "FORBIDDEN",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Conflict(_) => "CONFLICT",
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            )
            | Self::Admission(_)
            | Self::Database(_)
            | Self::InvalidStoredResourceId(_) => "dependency unavailable",
            Self::Authentication(_) => "authentication required",
            Self::Forbidden => "resource unavailable",
            Self::InvalidInput(message) => message,
            Self::Conflict(message) => message,
        }
    }
}

impl std::fmt::Display for CommunityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.safe_message())
    }
}

impl std::error::Error for CommunityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Authentication(error) => Some(error),
            Self::Admission(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::InvalidStoredResourceId(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for CommunityError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<ResourceIdError> for CommunityError {
    fn from(error: ResourceIdError) -> Self {
        Self::InvalidStoredResourceId(error)
    }
}

impl From<super::write_admission::WriteAdmissionError> for CommunityError {
    fn from(error: super::write_admission::WriteAdmissionError) -> Self {
        Self::Admission(error)
    }
}

impl From<crate::auth::authority::AuthError> for CommunityError {
    fn from(error: crate::auth::authority::AuthError) -> Self {
        Self::Authentication(error)
    }
}

impl From<super::authorization::AuthorizationError> for CommunityError {
    fn from(error: super::authorization::AuthorizationError) -> Self {
        match error {
            super::authorization::AuthorizationError::Unavailable => Self::Forbidden,
            super::authorization::AuthorizationError::Database(error) => Self::Database(error),
            super::authorization::AuthorizationError::Authentication(error) => {
                Self::Authentication(error)
            }
        }
    }
}

impl From<CommunityError> for String {
    fn from(error: CommunityError) -> Self {
        format!("{}: {}", error.code(), error.safe_message())
    }
}

pub struct RedeemedInvite {
    pub server_id: ServerId,
}

pub struct CreatedInvite {
    pub id: String,
    pub code: String,
    pub created_at: String,
}

pub struct CreatedFollow {
    pub id: String,
    pub created_by: String,
}

pub struct UpdateCommunityParams<'a> {
    pub server_id: &'a ServerId,
    pub description: Option<&'a str>,
    pub discoverable: bool,
    pub welcome: Option<&'a str>,
    pub rules: Option<&'a str>,
    pub category: Option<&'a str>,
}

pub struct CreateEvent<'a> {
    pub id: &'a str,
    pub server_id: &'a ServerId,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub channel_id: Option<&'a ChannelId>,
    pub start_time: &'a str,
    pub end_time: Option<&'a str>,
    pub image_url: Option<&'a str>,
    pub created_by: &'a str,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateConfig {
    format_version: i64,
    channels: Vec<TemplateChannel>,
    categories: Vec<TemplateCategory>,
    roles: Vec<TemplateRole>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateChannel {
    id: String,
    name: String,
    topic: String,
    category_id: Option<String>,
    position: i32,
    is_private: bool,
    channel_type: String,
    slowmode_seconds: i32,
    is_nsfw: bool,
    is_announcement: bool,
    is_default: bool,
    aliases: Vec<String>,
    role_overrides: Vec<TemplateRoleOverride>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateCategory {
    id: String,
    name: String,
    position: i32,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateRole {
    id: String,
    name: String,
    color: Option<String>,
    position: i32,
    permissions: i64,
    is_default: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateRoleOverride {
    role_id: String,
    allow_bits: i64,
    deny_bits: i64,
}

pub struct CreatedTemplate {
    pub id: String,
    pub created_at: String,
}

pub struct VisibleEvent {
    pub event: ServerEventRow,
    pub rsvp_count: i64,
}

impl TemplateConfig {
    fn validate(&self) -> Result<(), CommunityError> {
        if self.format_version != 1
            || self.channels.len() > 500
            || self.categories.len() > 100
            || self.roles.len() > 250
        {
            return Err(CommunityError::InvalidInput("invalid server template"));
        }
        let category_ids: HashSet<&str> = self
            .categories
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        let channel_ids: HashSet<&str> =
            self.channels.iter().map(|item| item.id.as_str()).collect();
        let role_ids: HashSet<&str> = self.roles.iter().map(|item| item.id.as_str()).collect();
        if category_ids.len() != self.categories.len()
            || channel_ids.len() != self.channels.len()
            || role_ids.len() != self.roles.len()
            || self.roles.iter().filter(|role| role.is_default).count() != 1
            || self
                .channels
                .iter()
                .filter(|channel| channel.is_default)
                .count()
                != 1
        {
            return Err(CommunityError::InvalidInput("invalid server template"));
        }
        for category in &self.categories {
            crate::engine::validation::validate_server_name(&category.name)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
            if category.position < 0 {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
        }
        let mut aliases = HashSet::new();
        for channel in &self.channels {
            crate::engine::validation::validate_channel_name(&channel.name)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
            crate::engine::validation::validate_topic(&channel.topic)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
            if channel.position < 0
                || !(0..=21_600).contains(&channel.slowmode_seconds)
                || !matches!(channel.channel_type.as_str(), "text" | "forum")
                || channel
                    .category_id
                    .as_deref()
                    .is_some_and(|id| !category_ids.contains(id))
            {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
            let mut override_roles = HashSet::new();
            if channel.aliases.iter().any(|alias| {
                alias.is_empty()
                    || alias.len() > 100
                    || !alias.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                    })
                    || !aliases.insert(alias.to_ascii_lowercase())
            }) || channel.role_overrides.iter().any(|rule| {
                !role_ids.contains(rule.role_id.as_str())
                    || !override_roles.insert(rule.role_id.as_str())
                    || rule.allow_bits < 0
                    || rule.deny_bits < 0
                    || Permissions::from_bits(rule.allow_bits as u64).is_none()
                    || Permissions::from_bits(rule.deny_bits as u64).is_none()
            }) {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
            if channel.is_default && channel.is_private {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
        }
        for role in &self.roles {
            if role.id.is_empty()
                || role.name.trim().is_empty()
                || role.name.len() > 100
                || role.position < 0
                || role.permissions < 0
                || Permissions::from_bits(role.permissions as u64).is_none()
            {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct CommunityService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

impl CommunityService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self {
            pool: pool.clone(),
            authorization: AuthorizationService::new(pool.clone()),
            auth,
            writes,
        }
    }
}

#[cfg(test)]
mod tests;

mod announcements;
mod discovery;
mod event_participation;
mod events;
mod invite_redemption;
mod invites;
mod template_creation;
mod template_instantiation;
