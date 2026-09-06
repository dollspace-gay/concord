use std::fmt;

use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool};

use crate::auth::authority::{Actor, AuthService, CredentialKind};

use crate::db::models::ChannelRow;

use crate::db::models::MessageRow;

use crate::db::models::ServerMemberRow;

use crate::engine::permissions::{
    ChannelOverride, OverrideTargetType, Permissions, ServerRole, compute_effective_permissions,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelAction {
    View,
    ReadHistory,
    Send,
    Manage,
    ManageMessages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationAction {
    View,
    Read,
    Send,
    ManageMessages,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationStamp {
    pub server_id: String,
    pub server_version: i64,
    pub channel_versions: Vec<(String, i64)>,
}

#[cfg(test)]
mod tests;

impl ChannelAction {
    fn permission(self) -> Permissions {
        match self {
            Self::View => Permissions::VIEW_CHANNELS,
            Self::ReadHistory => Permissions::READ_MESSAGE_HISTORY,
            Self::Send => Permissions::SEND_MESSAGES,
            Self::Manage => Permissions::MANAGE_CHANNELS,
            Self::ManageMessages => Permissions::MANAGE_MESSAGES,
        }
    }
}

#[derive(Debug)]
pub enum AuthorizationError {
    Unavailable,
    Database(sqlx::Error),
    Authentication(crate::auth::authority::AuthError),
}

impl fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("resource unavailable"),
            Self::Database(_) => formatter.write_str("authorization database operation failed"),
            Self::Authentication(error) => write!(formatter, "authentication failed: {error}"),
        }
    }
}

impl std::error::Error for AuthorizationError {}

impl From<sqlx::Error> for AuthorizationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Clone)]
pub struct AuthorizationService {
    pool: SqlitePool,
}

pub struct MessageSearch<'a> {
    pub server_id: &'a str,
    pub query: Option<&'a str>,
    pub requested_channel_id: Option<&'a str>,
    pub sender: Option<&'a str>,
    pub has_attachment: bool,
    pub has_link: bool,
    pub before: Option<&'a str>,
    pub after: Option<&'a str>,
    pub after_inclusive: bool,
    pub limit: i64,
    pub offset: i64,
    pub cursor_created_at: Option<&'a str>,
    pub cursor_message_id: Option<&'a str>,
}

struct ServerAuthority {
    owner_id: String,
    role_permissions: Vec<(String, Permissions)>,
    default_role_id: String,
    base_permissions: Permissions,
    privileged: bool,
}

struct ActorScopeRequirement<'a> {
    server_id: &'a str,
    scope: &'a str,
    channel_id: Option<&'a str>,
    allow_exact_channel: bool,
}

impl AuthorizationService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

mod actor_scope;
mod channel_policy;
mod conversations;
mod search;
mod stamps;
mod visibility;
