use std::fmt;

use sqlx::SqlitePool;

use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService, CredentialId, UserId};

use crate::db::models::{CreateAuditLogParams, WebhookRow};

use crate::engine::authorization::AuthorizationService;

use crate::engine::permissions::Permissions;

use crate::engine::write_admission::WriteAdmission;

#[derive(Debug)]
pub enum IntegrationError {
    Unavailable,
    Forbidden,
    DependencyUnavailable,
    InvalidInput(&'static str),
    Database(sqlx::Error),
    Authentication(crate::auth::authority::AuthError),
}

impl fmt::Display for IntegrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("resource unavailable"),
            Self::Forbidden => formatter.write_str("FORBIDDEN: integration operation denied"),
            Self::DependencyUnavailable => formatter
                .write_str("DEPENDENCY_UNAVAILABLE: integration write admission unavailable"),
            Self::InvalidInput(message) => write!(formatter, "INVALID_INPUT: {message}"),
            Self::Database(_) => {
                formatter.write_str("DEPENDENCY_UNAVAILABLE: integration database unavailable")
            }
            Self::Authentication(_) => {
                formatter.write_str("AUTHENTICATION_REQUIRED: credential is no longer valid")
            }
        }
    }
}

impl std::error::Error for IntegrationError {}

impl From<sqlx::Error> for IntegrationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<crate::auth::authority::AuthError> for IntegrationError {
    fn from(error: crate::auth::authority::AuthError) -> Self {
        match error {
            crate::auth::authority::AuthError::VerificationBusy
            | crate::auth::authority::AuthError::HashWorker(_) => Self::DependencyUnavailable,
            crate::auth::authority::AuthError::Database(error) => Self::Database(error),
            error => Self::Authentication(error),
        }
    }
}

impl From<crate::engine::authorization::AuthorizationError> for IntegrationError {
    fn from(error: crate::engine::authorization::AuthorizationError) -> Self {
        match error {
            crate::engine::authorization::AuthorizationError::Unavailable => Self::Forbidden,
            crate::engine::authorization::AuthorizationError::Database(error) => {
                Self::Database(error)
            }
            crate::engine::authorization::AuthorizationError::Authentication(error) => {
                Self::Authentication(error)
            }
        }
    }
}

impl From<crate::engine::write_admission::WriteAdmissionError> for IntegrationError {
    fn from(error: crate::engine::write_admission::WriteAdmissionError) -> Self {
        match error {
            crate::engine::write_admission::WriteAdmissionError::Unavailable => {
                Self::DependencyUnavailable
            }
            crate::engine::write_admission::WriteAdmissionError::Database(error) => {
                Self::Database(error)
            }
        }
    }
}

pub struct CreateWebhook<'a> {
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub name: &'a str,
    pub webhook_type: &'a str,
    pub url: Option<&'a str>,
}

pub struct CreatedWebhook {
    pub row: WebhookRow,
    pub one_time_secret: String,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct WebhookDeliveryStatus {
    pub delivery_id: String,
    pub event_type: String,
    pub event_version: i64,
    pub state: String,
    pub attempt_count: i64,
    pub last_status: Option<i64>,
    pub safe_error_code: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

#[derive(Clone)]
pub struct IntegrationService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: WriteAdmission,
    vault: std::sync::Arc<crate::secrets::SecretVault>,
}

impl IntegrationService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: WriteAdmission,
        vault: std::sync::Arc<crate::secrets::SecretVault>,
    ) -> Self {
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            pool,
            auth,
            writes,
            vault,
        }
    }
}

#[cfg(test)]
mod tests;

mod deliveries;
mod webhook_creation;
mod webhook_management;
