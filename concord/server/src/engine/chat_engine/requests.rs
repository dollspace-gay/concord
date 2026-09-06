pub struct CreateAutomodRuleRequest<'a> {
    pub server_id: &'a str,
    pub name: &'a str,
    pub rule_type: &'a str,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

pub struct UpdateAutomodRuleRequest<'a> {
    pub rule_id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

pub struct CreateServerEventRequest<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub channel_id: Option<&'a str>,
    pub start_time: &'a str,
    pub end_time: Option<&'a str>,
    pub image_url: Option<&'a str>,
    pub created_by: &'a str,
}

#[derive(Debug)]
pub enum AtprotoPublicationError {
    Unavailable,
    Authentication,
    DependencyUnavailable,
    Database(sqlx::Error),
}

impl std::fmt::Display for AtprotoPublicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("publication unavailable"),
            Self::Authentication => formatter.write_str("publication authentication required"),
            Self::DependencyUnavailable | Self::Database(_) => {
                formatter.write_str("publication dependency unavailable")
            }
        }
    }
}

impl std::error::Error for AtprotoPublicationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<crate::db::queries::atproto::PublicationRequestError> for AtprotoPublicationError {
    fn from(error: crate::db::queries::atproto::PublicationRequestError) -> Self {
        match error {
            crate::db::queries::atproto::PublicationRequestError::Unavailable => Self::Unavailable,
            crate::db::queries::atproto::PublicationRequestError::Authentication => {
                Self::Authentication
            }
            crate::db::queries::atproto::PublicationRequestError::DependencyUnavailable => {
                Self::DependencyUnavailable
            }
            crate::db::queries::atproto::PublicationRequestError::Database(error) => {
                Self::Database(error)
            }
        }
    }
}

/// The default server ID used as a fallback for IRC clients
/// that don't specify a server. No server with this ID is pre-created;
/// IRC bare-channel operations will fail unless one is created by a user.
pub const DEFAULT_SERVER_ID: &str = "default";

/// Parameters for updating notification settings (avoids too-many-arguments).
pub struct UpdateNotificationSettingsParams<'a> {
    pub server_id: &'a str,
    pub channel_id: Option<&'a str>,
    pub level: &'a str,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    pub mute_until: Option<&'a str>,
}

#[derive(sqlx::FromRow)]
pub(super) struct PresenceProjectionRow {
    pub(super) user_id: String,
    pub(super) nickname: String,
    pub(super) avatar_url: Option<String>,
    pub(super) requested_status: Option<String>,
    pub(super) custom_status: Option<String>,
    pub(super) status_emoji: Option<String>,
}

pub enum Synchronization {
    Snapshot(crate::engine::replay::SyncSnapshot),
    Replay(crate::engine::replay::ReplayBatch),
}
