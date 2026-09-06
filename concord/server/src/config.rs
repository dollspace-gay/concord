use std::env;

use std::fmt;

use std::fs::{self, OpenOptions};

use std::io::Write;

use std::net::SocketAddr;

use std::path::{Path, PathBuf};

use rand::Rng;

use serde::Deserialize;

use sqlx::SqlitePool;

use url::Url;

use crate::auth::config::AuthConfig;

const MIN_SECRET_BYTES: usize = 32;

const MAX_SESSION_EXPIRY_HOURS: i64 = 8_760;

const MAX_FILE_SIZE_MB: u64 = 10_240;

const MAX_MESSAGE_LENGTH: usize = 1_000_000;

const MAX_SHUTDOWN_SECONDS: u64 = 300;

const SAMPLE_SECRETS: &[&str] = &[
    "concord-dev-secret-change-me",
    "change-me-to-a-random-secret",
    "your-secret-here",
    "changeme",
];

#[derive(Debug)]
pub enum ConfigError {
    Read {
        field: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Write {
        field: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    Invalid {
        field: &'static str,
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read {
                field,
                path,
                source,
            } => write!(
                formatter,
                "cannot read configuration field {field} from {}: {source}",
                path.display()
            ),
            Self::Write {
                field,
                path,
                source,
            } => write!(
                formatter,
                "cannot initialize configuration field {field} at {}: {source}",
                path.display()
            ),
            Self::Parse { path, line, column } => write!(
                formatter,
                "cannot parse configuration {} at line {line}, column {column}; values are not shown",
                path.display(),
            ),
            Self::Invalid { field, reason } => {
                write!(formatter, "invalid configuration field {field}: {reason}")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            Self::Parse { .. } | Self::Invalid { .. } => None,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub server: ServerSection,
    pub database: DatabaseSection,
    pub auth: AuthSection,
    pub storage: StorageSection,
    pub admin: AdminSection,
    pub irc: IrcSection,
    pub egress: EgressSection,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct EgressSection {
    /// Exact origins available only to operator/admin integrations.
    pub operator_allowed_origins: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AdminSection {
    /// Stable user IDs (AT Protocol DIDs for the current login provider).
    pub admin_user_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSection {
    pub web_address: String,
    pub irc_address: String,
    pub irc_tls_cert: Option<PathBuf>,
    pub irc_tls_key: Option<PathBuf>,
    pub shutdown_timeout_seconds: u64,
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            web_address: "0.0.0.0:8080".into(),
            irc_address: "127.0.0.1:6667".into(),
            irc_tls_cert: None,
            irc_tls_key: None,
            shutdown_timeout_seconds: 30,
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseSection {
    pub url: String,
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            url: "sqlite:data/concord.db?mode=rwc".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthSection {
    /// Inline secrets are retained for environment compatibility; prefer `jwt_secret_file`.
    pub jwt_secret: String,
    pub jwt_secret_file: Option<PathBuf>,
    pub external_credentials_key_file: PathBuf,
    pub session_expiry_hours: i64,
    pub public_url: String,
}

impl Default for AuthSection {
    fn default() -> Self {
        Self {
            jwt_secret: String::new(),
            jwt_secret_file: None,
            external_credentials_key_file: "data/secrets/external-credentials.key".into(),
            session_expiry_hours: 720,
            public_url: "http://localhost:8080".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageSection {
    pub data_dir: PathBuf,
    pub media_dir: PathBuf,
    pub max_file_size_mb: u64,
    pub max_media_per_user_mb: u64,
    pub max_media_total_mb: u64,
    pub max_message_length: usize,
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            data_dir: "data".into(),
            media_dir: "data/media".into(),
            max_file_size_mb: 100,
            max_media_per_user_mb: 10_240,
            max_media_total_mb: 102_400,
            max_message_length: 4_000,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct IrcSection {
    pub motd: Vec<String>,
}

impl ServerConfig {
    /// Load exactly one TOML/environment snapshot and reject unsafe or unusable values.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::load_with_env(path.as_ref(), |name| env::var(name).ok())
    }
    /// Load storage/database settings for recovery commands even when the
    /// external credential key is the failed component being investigated.
    pub fn load_for_recovery(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::load_with_env_mode(path.as_ref(), |name| env::var(name).ok(), false)
    }
    fn load_with_env(
        path: &Path,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        Self::load_with_env_mode(path, environment, true)
    }
    fn load_with_env_mode(
        path: &Path,
        environment: impl Fn(&str) -> Option<String>,
        validate_external_key: bool,
    ) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            field: "config_file",
            path: path.to_path_buf(),
            source,
        })?;
        let mut config: Self = toml::from_str(&contents).map_err(|source| {
            let offset = source
                .span()
                .map_or(0, |span| span.start.min(contents.len()));
            let prefix = &contents[..offset];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = prefix
                .rsplit_once('\n')
                .map_or(prefix.len(), |(_, tail)| tail.len())
                + 1;
            ConfigError::Parse {
                path: path.to_path_buf(),
                line,
                column,
            }
        })?;
        config.apply_env_overrides(environment)?;
        config.resolve_paths(
            path.parent().unwrap_or_else(|| Path::new(".")),
            validate_external_key,
        )?;
        config.validate()?;
        Ok(config)
    }
}

/// Idempotently grant system administration to a verified stable identity.
pub async fn ensure_configured_admin(
    pool: &SqlitePool,
    user_id: &str,
    configured_ids: &[String],
) -> Result<bool, sqlx::Error> {
    if !configured_ids.iter().any(|candidate| candidate == user_id) {
        return Ok(false);
    }
    let result = sqlx::query("UPDATE users SET is_system_admin = 1 WHERE id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

#[cfg(test)]
mod tests;

mod environment;

mod private_files;

mod validation;

use environment::invalid;

use environment::parse_env;

use environment::resolve_database_url;

use environment::resolve_optional_path;

use environment::resolve_path;

use environment::set_path;

use environment::set_string;

use private_files::create_private_dir;

use private_files::write_new_private_file;

use validation::bounded;

pub use validation::is_loopback_public_url;

use validation::parse_socket;

use validation::validate_admin_ids;

use validation::validate_database_parent;

use validation::validate_private_file;

use validation::validate_public_url;

use validation::validate_secret;

use validation::validate_tls_pair;

use validation::validate_writable_dir;

mod configuration;
mod initialization;
