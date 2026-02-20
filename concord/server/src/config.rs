use serde::Deserialize;
use std::path::Path;
use tracing::info;

use crate::auth::config::AuthConfig;

/// Top-level server configuration, loaded from concord.toml.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ServerConfig {
    pub server: ServerSection,
    pub database: DatabaseSection,
    pub auth: AuthSection,
    pub storage: StorageSection,
    pub admin: AdminSection,
    pub irc: IrcSection,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AdminSection {
    /// Usernames that should be auto-promoted to system admin on startup.
    pub admin_users: Vec<String>,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct ServerSection {
    pub web_address: String,
    pub irc_address: String,
    /// Path to TLS certificate file (PEM) for IRC. If set, IRC listens with TLS.
    pub irc_tls_cert: Option<String>,
    /// Path to TLS private key file (PEM) for IRC.
    pub irc_tls_key: Option<String>,
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            web_address: "0.0.0.0:8080".into(),
            irc_address: "0.0.0.0:6667".into(),
            irc_tls_cert: None,
            irc_tls_key: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct DatabaseSection {
    pub url: String,
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            url: "sqlite:concord.db?mode=rwc".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct AuthSection {
    pub jwt_secret: String,
    pub session_expiry_hours: i64,
    pub public_url: String,
}

impl Default for AuthSection {
    fn default() -> Self {
        Self {
            jwt_secret: "concord-dev-secret-change-me".into(),
            session_expiry_hours: 720,
            public_url: "http://localhost:8080".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct StorageSection {
    pub max_file_size_mb: u64,
    pub max_message_length: usize,
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            max_file_size_mb: 100,
            max_message_length: 4000,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct IrcSection {
    /// Message of the day lines. If empty, clients see ERR_NOMOTD.
    pub motd: Vec<String>,
}

impl ServerConfig {
    /// Load config from a TOML file. Falls back to defaults if the file doesn't exist.
    /// Environment variables override TOML values.
    pub fn load(path: &str) -> Self {
        let mut config = if Path::new(path).exists() {
            let contents = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("failed to read config file {}: {}", path, e));
            toml::from_str(&contents)
                .unwrap_or_else(|e| panic!("failed to parse config file {}: {}", path, e))
        } else {
            info!("No config file found at {}, using defaults", path);
            Self::default()
        };

        config.apply_env_overrides();
        config
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("WEB_ADDRESS") {
            self.server.web_address = v;
        }
        if let Ok(v) = std::env::var("IRC_ADDRESS") {
            self.server.irc_address = v;
        }
        if let Ok(v) = std::env::var("IRC_TLS_CERT") {
            self.server.irc_tls_cert = Some(v);
        }
        if let Ok(v) = std::env::var("IRC_TLS_KEY") {
            self.server.irc_tls_key = Some(v);
        }
        if let Ok(v) = std::env::var("DATABASE_URL") {
            self.database.url = v;
        }
        if let Ok(v) = std::env::var("JWT_SECRET") {
            self.auth.jwt_secret = v;
        }
        if let Ok(v) = std::env::var("SESSION_EXPIRY_HOURS")
            && let Ok(hours) = v.parse()
        {
            self.auth.session_expiry_hours = hours;
        }
        if let Ok(v) = std::env::var("PUBLIC_URL") {
            self.auth.public_url = v;
        }
        if let Ok(v) = std::env::var("MAX_FILE_SIZE_MB")
            && let Ok(mb) = v.parse()
        {
            self.storage.max_file_size_mb = mb;
        }
        if let Ok(v) = std::env::var("MAX_MESSAGE_LENGTH")
            && let Ok(len) = v.parse()
        {
            self.storage.max_message_length = len;
        }
        if let Ok(v) = std::env::var("ADMIN_USERS") {
            self.admin.admin_users = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    /// Convert into an AuthConfig for the auth layer.
    pub fn to_auth_config(&self) -> AuthConfig {
        AuthConfig {
            jwt_secret: self.auth.jwt_secret.clone(),
            session_expiry_hours: self.auth.session_expiry_hours,
            public_url: self.auth.public_url.clone(),
        }
    }
}
