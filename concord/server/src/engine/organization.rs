use sqlx::{Row, SqlitePool};

use crate::auth::authority::{Actor, AuthService};

use crate::db::models::RoleRow;

use super::authorization::{AuthorizationService, ChannelAction};

use super::events::{CategoryInfo, ChannelPermissionOverrideInfo, ChannelPositionInfo, ServerInfo};

use super::ids::{ChannelId, ServerId};

use super::permissions::Permissions;

pub struct ProvisionedServer {
    pub server_id: String,
    pub channel_id: String,
    pub server_alias: String,
}

pub struct ServerMemberSummary {
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}

pub struct CreateChannel<'a> {
    pub server_id: &'a ServerId,
    pub channel_id: &'a ChannelId,
    pub name: &'a str,
    pub category_id: Option<&'a str>,
    pub is_private: bool,
    pub channel_type: &'a str,
}

pub struct ChannelOverrideUpdate<'a> {
    pub server_id: &'a ServerId,
    pub channel_id: &'a ChannelId,
    pub target_type: &'a str,
    pub target_id: &'a str,
    pub allow_bits: i64,
    pub deny_bits: i64,
}

#[derive(Debug)]
pub enum OrganizationError {
    Authentication(crate::auth::authority::AuthError),
    Forbidden,
    InvalidInput(&'static str),
    Admission(super::write_admission::WriteAdmissionError),
    Database(sqlx::Error),
}

impl OrganizationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ) => "DEPENDENCY_UNAVAILABLE",
            Self::Authentication(_) => "UNAUTHENTICATED",
            Self::Forbidden => "FORBIDDEN",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Admission(_) | Self::Database(_) => "DEPENDENCY_UNAVAILABLE",
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ) => "dependency unavailable",
            Self::Authentication(_) => "authentication required",
            Self::Forbidden => "resource unavailable",
            Self::InvalidInput(message) => message,
            Self::Admission(_) | Self::Database(_) => "dependency unavailable",
        }
    }
}

impl std::fmt::Display for OrganizationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.safe_message())
    }
}

impl std::error::Error for OrganizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::Authentication(error) => Some(error),
            _ => None,
        }
    }
}

impl From<OrganizationError> for String {
    fn from(error: OrganizationError) -> Self {
        format!("{}: {}", error.code(), error.safe_message())
    }
}

impl From<sqlx::Error> for OrganizationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<super::write_admission::WriteAdmissionError> for OrganizationError {
    fn from(error: super::write_admission::WriteAdmissionError) -> Self {
        Self::Admission(error)
    }
}

impl From<super::authorization::AuthorizationError> for OrganizationError {
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

#[derive(Clone)]
pub struct OrganizationService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

impl OrganizationService {
    const CHANNEL_OVERRIDE_PERMISSIONS: Permissions = Permissions::VIEW_CHANNELS
        .union(Permissions::MANAGE_CHANNELS)
        .union(Permissions::SEND_MESSAGES)
        .union(Permissions::EMBED_LINKS)
        .union(Permissions::ATTACH_FILES)
        .union(Permissions::ADD_REACTIONS)
        .union(Permissions::MENTION_EVERYONE)
        .union(Permissions::MANAGE_MESSAGES)
        .union(Permissions::READ_MESSAGE_HISTORY)
        .union(Permissions::CONNECT)
        .union(Permissions::SPEAK)
        .union(Permissions::MUTE_MEMBERS)
        .union(Permissions::DEAFEN_MEMBERS)
        .union(Permissions::MOVE_MEMBERS);
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

async fn bump_role_projection_version(
    connection: &mut sqlx::SqliteConnection,
    server_id: &str,
) -> Result<(), OrganizationError> {
    sqlx::query("UPDATE servers SET role_projection_version=role_projection_version+1 WHERE id=?")
        .bind(server_id)
        .execute(connection)
        .await
        .map_err(OrganizationError::from)?;
    Ok(())
}

fn require_role_grant(actor_permissions: Permissions, bits: i64) -> Result<(), OrganizationError> {
    if bits < 0 {
        return Err(OrganizationError::InvalidInput("invalid role permissions"));
    }
    let requested = Permissions::from_bits(bits as u64).ok_or(OrganizationError::Forbidden)?;
    if !actor_permissions.contains(Permissions::MANAGE_ROLES)
        || !actor_permissions.contains(requested)
    {
        return Err(OrganizationError::Forbidden);
    }
    Ok(())
}

fn validate_role_fields(name: &str, color: Option<&str>) -> Result<(), OrganizationError> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.len() > 100
        || name.chars().any(char::is_control)
    {
        return Err(OrganizationError::InvalidInput("invalid role name"));
    }
    if color.is_some_and(|value| {
        value.len() != 7
            || !value.starts_with('#')
            || !value[1..]
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    }) {
        return Err(OrganizationError::InvalidInput("invalid role color"));
    }
    Ok(())
}

async fn highest_role_position(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    server_id: &str,
    user_id: &str,
    owner: bool,
) -> Result<i32, OrganizationError> {
    if owner {
        return Ok(i32::MAX);
    }
    sqlx::query_scalar("SELECT COALESCE(MAX(r.position),0) FROM roles r JOIN user_roles ur ON ur.role_id=r.id AND ur.server_id=r.server_id WHERE r.server_id=? AND ur.user_id=?")
        .bind(server_id).bind(user_id).fetch_one(&mut **tx).await.map_err(OrganizationError::from)
}

async fn require_managed_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &Actor,
    server_id: &str,
    role_id: &str,
) -> Result<(), OrganizationError> {
    let owner: bool = sqlx::query_scalar("SELECT owner_id=? FROM servers WHERE id=?")
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(OrganizationError::from)?
        .ok_or(OrganizationError::Forbidden)?;
    let role: Option<(i32, i32)> =
        sqlx::query_as("SELECT position,is_default FROM roles WHERE id=? AND server_id=?")
            .bind(role_id)
            .bind(server_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(OrganizationError::from)?;
    let (position, is_default) = role.ok_or(OrganizationError::Forbidden)?;
    if is_default != 0 {
        return Err(OrganizationError::Forbidden);
    }
    let actor_highest =
        highest_role_position(tx, server_id, actor.user_id().as_str(), owner).await?;
    if !owner && actor_highest <= position {
        return Err(OrganizationError::Forbidden);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

mod administration;
mod categories;
mod channel_overrides;
mod channels;
mod membership;
mod roles;
mod servers;
