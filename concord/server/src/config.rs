use std::env;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rand::RngCore;
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

    /// Create a new configuration, persistent secret, and writable data directories.
    /// Existing files are never replaced.
    pub fn initialize(path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if path.exists() {
            return Err(ConfigError::Write {
                field: "config_file",
                path: path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "configuration already exists",
                ),
            });
        }
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let data_dir = root.join("data");
        let media_dir = data_dir.join("media");
        let secret_dir = data_dir.join("secrets");
        let secret_path = secret_dir.join("jwt.key");
        let external_key_path = secret_dir.join("external-credentials.key");
        create_private_dir(&data_dir, "storage.data_dir")?;
        create_private_dir(&media_dir, "storage.media_dir")?;
        create_private_dir(&secret_dir, "auth.jwt_secret_file")?;

        let mut random = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut random);
        let secret = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        write_new_private_file(&secret_path, secret.as_bytes(), "auth.jwt_secret_file")?;
        rand::thread_rng().fill_bytes(&mut random);
        let external_key = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        write_new_private_file(
            &external_key_path,
            external_key.as_bytes(),
            "auth.external_credentials_key_file",
        )?;
        let template = r#"[server]
web_address = "0.0.0.0:8080"
irc_address = "127.0.0.1:6667"
shutdown_timeout_seconds = 30

[database]
url = "sqlite:data/concord.db?mode=rwc"

[auth]
jwt_secret_file = "data/secrets/jwt.key"
external_credentials_key_file = "data/secrets/external-credentials.key"
session_expiry_hours = 720
public_url = "http://localhost:8080"

[storage]
data_dir = "data"
media_dir = "data/media"
max_file_size_mb = 100
max_media_per_user_mb = 10240
max_media_total_mb = 102400
max_message_length = 4000

[admin]
admin_user_ids = []

[irc]
motd = []

[egress]
operator_allowed_origins = []
"#;
        write_new_private_file(path, template.as_bytes(), "config_file")
    }

    pub fn to_auth_config(&self) -> AuthConfig {
        AuthConfig {
            jwt_secret: self.auth.jwt_secret.clone(),
            session_expiry_hours: self.auth.session_expiry_hours,
            public_url: self.auth.public_url.clone(),
        }
    }

    fn apply_env_overrides(
        &mut self,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<(), ConfigError> {
        set_string(&environment, "WEB_ADDRESS", &mut self.server.web_address);
        set_string(&environment, "IRC_ADDRESS", &mut self.server.irc_address);
        set_path(&environment, "IRC_TLS_CERT", &mut self.server.irc_tls_cert);
        set_path(&environment, "IRC_TLS_KEY", &mut self.server.irc_tls_key);
        set_string(&environment, "DATABASE_URL", &mut self.database.url);
        if let Some(value) = environment("JWT_SECRET") {
            self.auth.jwt_secret = value;
            self.auth.jwt_secret_file = None;
        }
        if let Some(value) = environment("JWT_SECRET_FILE") {
            self.auth.jwt_secret.clear();
            self.auth.jwt_secret_file = Some(value.into());
        }
        if let Some(value) = environment("EXTERNAL_CREDENTIALS_KEY_FILE") {
            self.auth.external_credentials_key_file = value.into();
        }
        set_string(&environment, "PUBLIC_URL", &mut self.auth.public_url);
        self.auth.session_expiry_hours = parse_env(
            &environment,
            "SESSION_EXPIRY_HOURS",
            "auth.session_expiry_hours",
            self.auth.session_expiry_hours,
        )?;
        self.storage.max_file_size_mb = parse_env(
            &environment,
            "MAX_FILE_SIZE_MB",
            "storage.max_file_size_mb",
            self.storage.max_file_size_mb,
        )?;
        self.storage.max_media_per_user_mb = parse_env(
            &environment,
            "MAX_MEDIA_PER_USER_MB",
            "storage.max_media_per_user_mb",
            self.storage.max_media_per_user_mb,
        )?;
        self.storage.max_media_total_mb = parse_env(
            &environment,
            "MAX_MEDIA_TOTAL_MB",
            "storage.max_media_total_mb",
            self.storage.max_media_total_mb,
        )?;
        self.storage.max_message_length = parse_env(
            &environment,
            "MAX_MESSAGE_LENGTH",
            "storage.max_message_length",
            self.storage.max_message_length,
        )?;
        self.server.shutdown_timeout_seconds = parse_env(
            &environment,
            "SHUTDOWN_TIMEOUT_SECONDS",
            "server.shutdown_timeout_seconds",
            self.server.shutdown_timeout_seconds,
        )?;
        if let Some(value) = environment("DATA_DIR") {
            self.storage.data_dir = value.into();
        }
        if let Some(value) = environment("MEDIA_DIR") {
            self.storage.media_dir = value.into();
        }
        if let Some(value) = environment("ADMIN_USER_IDS") {
            self.admin.admin_user_ids = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
        }
        Ok(())
    }

    fn resolve_paths(
        &mut self,
        root: &Path,
        validate_external_key: bool,
    ) -> Result<(), ConfigError> {
        resolve_optional_path(root, &mut self.server.irc_tls_cert);
        resolve_optional_path(root, &mut self.server.irc_tls_key);
        resolve_optional_path(root, &mut self.auth.jwt_secret_file);
        resolve_path(root, &mut self.auth.external_credentials_key_file);
        resolve_path(root, &mut self.storage.data_dir);
        resolve_path(root, &mut self.storage.media_dir);
        resolve_database_url(root, &mut self.database.url);
        if !self.auth.jwt_secret.is_empty() && self.auth.jwt_secret_file.is_some() {
            return invalid(
                "auth.jwt_secret",
                "set JWT_SECRET or jwt_secret_file, but not both",
            );
        }
        if let Some(secret_path) = &self.auth.jwt_secret_file {
            validate_private_file(secret_path, "auth.jwt_secret_file")?;
            self.auth.jwt_secret = fs::read_to_string(secret_path)
                .map_err(|source| ConfigError::Read {
                    field: "auth.jwt_secret_file",
                    path: secret_path.clone(),
                    source,
                })?
                .trim()
                .to_owned();
        }
        if validate_external_key {
            validate_private_file(
                &self.auth.external_credentials_key_file,
                "auth.external_credentials_key_file",
            )?;
        }
        Ok(())
    }

    fn validate(&mut self) -> Result<(), ConfigError> {
        parse_socket("server.web_address", &self.server.web_address)?;
        let irc_address = parse_socket("server.irc_address", &self.server.irc_address)?;
        bounded(
            "server.shutdown_timeout_seconds",
            self.server.shutdown_timeout_seconds,
            1,
            MAX_SHUTDOWN_SECONDS,
        )?;
        bounded(
            "auth.session_expiry_hours",
            self.auth.session_expiry_hours,
            1,
            MAX_SESSION_EXPIRY_HOURS,
        )?;
        bounded(
            "storage.max_file_size_mb",
            self.storage.max_file_size_mb,
            1,
            MAX_FILE_SIZE_MB,
        )?;
        bounded(
            "storage.max_media_per_user_mb",
            self.storage.max_media_per_user_mb,
            self.storage.max_file_size_mb,
            u64::MAX,
        )?;
        bounded(
            "storage.max_media_total_mb",
            self.storage.max_media_total_mb,
            self.storage.max_media_per_user_mb,
            u64::MAX,
        )?;
        bounded(
            "storage.max_message_length",
            self.storage.max_message_length,
            1,
            MAX_MESSAGE_LENGTH,
        )?;
        validate_secret(&self.auth.jwt_secret)?;
        self.auth.public_url = validate_public_url(&self.auth.public_url)?;
        validate_tls_pair(&self.server.irc_tls_cert, &self.server.irc_tls_key)?;
        if !irc_address.ip().is_loopback() && self.server.irc_tls_cert.is_none() {
            return invalid(
                "server.irc_address",
                "a non-loopback IRC listener requires TLS certificate and key",
            );
        }
        validate_writable_dir(&self.storage.data_dir, "storage.data_dir")?;
        validate_writable_dir(&self.storage.media_dir, "storage.media_dir")?;
        validate_database_parent(&self.database.url)?;
        validate_admin_ids(&self.admin.admin_user_ids)
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

pub fn is_loopback_public_url(value: &str) -> bool {
    Url::parse(value)
        .is_ok_and(|url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
}

fn validate_public_url(value: &str) -> Result<String, ConfigError> {
    let url = Url::parse(value).map_err(|_| ConfigError::Invalid {
        field: "auth.public_url",
        reason: "must be an absolute http or https origin".into(),
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return invalid(
            "auth.public_url",
            "must contain only an http or https scheme, host, and optional port",
        );
    }
    if url.scheme() == "http" && !is_loopback_public_url(value) {
        return invalid("auth.public_url", "non-loopback deployments require https");
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn validate_secret(secret: &str) -> Result<(), ConfigError> {
    if secret.len() < MIN_SECRET_BYTES {
        return invalid(
            "auth.jwt_secret",
            format!("must contain at least {MIN_SECRET_BYTES} bytes"),
        );
    }
    let lowercase = secret.to_ascii_lowercase();
    if SAMPLE_SECRETS
        .iter()
        .any(|sample| lowercase == *sample || lowercase.contains(sample))
    {
        return invalid("auth.jwt_secret", "known sample secrets are forbidden");
    }
    if secret.as_bytes().windows(2).all(|pair| pair[0] == pair[1]) {
        return invalid("auth.jwt_secret", "repeated-byte secrets are forbidden");
    }
    Ok(())
}

fn validate_tls_pair(cert: &Option<PathBuf>, key: &Option<PathBuf>) -> Result<(), ConfigError> {
    match (cert, key) {
        (Some(cert), Some(key)) => {
            validate_readable_file(cert, "server.irc_tls_cert")?;
            validate_private_file(key, "server.irc_tls_key")
        }
        (None, None) => Ok(()),
        _ => invalid(
            "server.irc_tls_cert/server.irc_tls_key",
            "certificate and private key must be configured together",
        ),
    }
}

fn validate_admin_ids(ids: &[String]) -> Result<(), ConfigError> {
    for (index, id) in ids.iter().enumerate() {
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return invalid(
                "admin.admin_user_ids",
                format!("entry {index} is not a stable user ID"),
            );
        }
        if ids[..index].contains(id) {
            return invalid(
                "admin.admin_user_ids",
                format!("entry {index} duplicates an earlier stable user ID"),
            );
        }
    }
    Ok(())
}

fn validate_database_parent(url: &str) -> Result<(), ConfigError> {
    let Some(path) = url.strip_prefix("sqlite:") else {
        return invalid("database.url", "only sqlite: URLs are supported");
    };
    let path = path.split('?').next().unwrap_or_default();
    if path.is_empty() || path == ":memory:" {
        return invalid("database.url", "must name a persistent SQLite database");
    }
    let database_path = Path::new(path.trim_start_matches("//"));
    validate_writable_dir(
        database_path.parent().unwrap_or_else(|| Path::new(".")),
        "database.url",
    )
}

fn validate_writable_dir(path: &Path, field: &'static str) -> Result<(), ConfigError> {
    let metadata = fs::metadata(path).map_err(|source| ConfigError::Read {
        field,
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return invalid(field, format!("{} is not a directory", path.display()));
    }
    if metadata.permissions().readonly() {
        return invalid(field, format!("{} is read-only", path.display()));
    }
    let probe = path.join(format!(".concord-write-probe-{}", std::process::id()));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            fs::remove_file(&probe).map_err(|source| ConfigError::Write {
                field,
                path: probe,
                source,
            })
        }
        Err(source) => Err(ConfigError::Write {
            field,
            path: probe,
            source,
        }),
    }
}

fn validate_private_file(path: &Path, field: &'static str) -> Result<(), ConfigError> {
    validate_readable_file(path, field)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|source| ConfigError::Read {
                field,
                path: path.to_path_buf(),
                source,
            })?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return invalid(
                field,
                "secret file must not be accessible by group or others",
            );
        }
    }
    Ok(())
}

fn validate_readable_file(path: &Path, field: &'static str) -> Result<(), ConfigError> {
    let metadata = fs::metadata(path).map_err(|source| ConfigError::Read {
        field,
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return invalid(field, format!("{} is not a regular file", path.display()));
    }
    fs::File::open(path)
        .map(|_| ())
        .map_err(|source| ConfigError::Read {
            field,
            path: path.to_path_buf(),
            source,
        })
}

fn parse_socket(field: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        field,
        reason: "must be an IP socket address such as 127.0.0.1:8080".into(),
    })
}

fn bounded<T: Copy + PartialOrd + fmt::Display>(
    field: &'static str,
    value: T,
    min: T,
    max: T,
) -> Result<(), ConfigError> {
    if value < min || value > max {
        return invalid(
            field,
            format!("must be between {min} and {max}, got {value}"),
        );
    }
    Ok(())
}

fn parse_env<T: std::str::FromStr>(
    environment: &impl Fn(&str) -> Option<String>,
    name: &str,
    field: &'static str,
    current: T,
) -> Result<T, ConfigError> {
    environment(name).map_or(Ok(current), |value| {
        value.parse().map_err(|_| ConfigError::Invalid {
            field,
            reason: format!("environment variable {name} has an invalid numeric value"),
        })
    })
}

fn set_string(environment: &impl Fn(&str) -> Option<String>, name: &str, target: &mut String) {
    if let Some(value) = environment(name) {
        *target = value;
    }
}
fn set_path(
    environment: &impl Fn(&str) -> Option<String>,
    name: &str,
    target: &mut Option<PathBuf>,
) {
    if let Some(value) = environment(name) {
        *target = Some(value.into());
    }
}
fn resolve_optional_path(root: &Path, path: &mut Option<PathBuf>) {
    if let Some(path) = path {
        resolve_path(root, path);
    }
}
fn resolve_path(root: &Path, path: &mut PathBuf) {
    if path.is_relative() {
        *path = root.join(&*path);
    }
}
fn resolve_database_url(root: &Path, value: &mut String) {
    let Some(rest) = value.strip_prefix("sqlite:") else {
        return;
    };
    let (path, query) = rest
        .split_once('?')
        .map_or((rest, None), |(path, query)| (path, Some(query)));
    if path == ":memory:" || path.is_empty() || Path::new(path).is_absolute() {
        return;
    }
    let resolved = root.join(path);
    *value = match query {
        Some(query) => format!("sqlite:{}?{query}", resolved.display()),
        None => format!("sqlite:{}", resolved.display()),
    };
}
fn invalid<T>(field: &'static str, reason: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid {
        field,
        reason: reason.into(),
    })
}

fn create_private_dir(path: &Path, field: &'static str) -> Result<(), ConfigError> {
    fs::create_dir_all(path).map_err(|source| ConfigError::Write {
        field,
        path: path.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            ConfigError::Write {
                field,
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn write_new_private_file(
    path: &Path,
    contents: &[u8],
    field: &'static str,
) -> Result<(), ConfigError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|source| ConfigError::Write {
        field,
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(contents)
        .map_err(|source| ConfigError::Write {
            field,
            path: path.to_path_buf(),
            source,
        })?;
    file.sync_all().map_err(|source| ConfigError::Write {
        field,
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fixture() -> PathBuf {
        let root = env::temp_dir().join(format!("concord-config-{}", Uuid::new_v4()));
        ServerConfig::initialize(root.join("concord.toml")).unwrap();
        root
    }

    fn load_without_env(path: &Path) -> Result<ServerConfig, ConfigError> {
        ServerConfig::load_with_env(path, |_| None)
    }

    #[test]
    fn explicit_initialization_persists_private_secret_and_valid_config() {
        let root = fixture();
        let config_path = root.join("concord.toml");
        let first_secret = fs::read_to_string(root.join("data/secrets/jwt.key")).unwrap();
        let config = load_without_env(&config_path).unwrap();
        assert_eq!(first_secret, config.auth.jwt_secret);
        assert!(first_secret.len() >= 64);
        assert!(ServerConfig::initialize(&config_path).is_err());
        assert_eq!(
            first_secret,
            fs::read_to_string(root.join("data/secrets/jwt.key")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_numeric_environment_instead_of_falling_back() {
        let root = fixture();
        let error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
            (name == "MAX_FILE_SIZE_MB").then(|| "many".into())
        })
        .err()
        .expect("invalid number must fail");
        assert!(error.to_string().contains("storage.max_file_size_mb"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_sample_secret_without_echoing_it() {
        let root = fixture();
        let sample = "change-me-to-a-random-secret";
        let error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
            (name == "JWT_SECRET").then(|| sample.into())
        })
        .err()
        .expect("sample secret must fail");
        assert!(error.to_string().contains("auth.jwt_secret"));
        assert!(!error.to_string().contains(sample));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_public_url_prefix_trick_and_partial_tls() {
        let root = fixture();
        let prefix_error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
            (name == "PUBLIC_URL").then(|| "http://localhost.attacker.example".into())
        })
        .err()
        .expect("prefix trick must fail");
        assert!(!is_loopback_public_url("http://localhost.attacker.example"));
        assert!(prefix_error.to_string().contains("auth.public_url"));
        let cert = root.join("certificate.pem");
        fs::write(&cert, "certificate").unwrap();
        let tls_error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
            (name == "IRC_TLS_CERT").then(|| cert.display().to_string())
        })
        .err()
        .expect("partial TLS must fail");
        assert!(tls_error.to_string().contains("irc_tls_cert"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_missing_storage_path() {
        let root = fixture();
        let error = ServerConfig::load_with_env(&root.join("concord.toml"), |name| {
            (name == "MEDIA_DIR").then(|| root.join("missing").display().to_string())
        })
        .err()
        .expect("missing media dir must fail");
        assert!(error.to_string().contains("storage.media_dir"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_toml_never_echoes_secret_source_text() {
        let root = env::temp_dir().join(format!("concord-config-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("concord.toml");
        let sentinel = "DO-NOT-LOG-THIS-SECRET";
        fs::write(&path, format!("[auth]\njwt_secret = \"{sentinel}\n")).unwrap();

        let error = load_without_env(&path)
            .err()
            .expect("malformed TOML must fail");
        let diagnostics = format!("{error}\n{error:?}");
        assert!(!diagnostics.contains(sentinel));
        assert!(diagnostics.contains("line"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn stable_id_admin_allowlist_applies_without_username_matching() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE users (id TEXT PRIMARY KEY, username TEXT NOT NULL, is_system_admin INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('did:plc:stable','mutable-handle')")
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            !ensure_configured_admin(&pool, "mutable-handle", &["did:plc:stable".into()])
                .await
                .unwrap()
        );
        assert!(
            ensure_configured_admin(&pool, "did:plc:stable", &["did:plc:stable".into()])
                .await
                .unwrap()
        );
        let is_admin: bool =
            sqlx::query_scalar("SELECT is_system_admin FROM users WHERE id='did:plc:stable'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(is_admin);
    }
}
