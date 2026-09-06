use super::{
    ConfigError, MAX_FILE_SIZE_MB, MAX_MESSAGE_LENGTH, MAX_SESSION_EXPIRY_HOURS,
    MAX_SHUTDOWN_SECONDS, Path, ServerConfig, bounded, invalid, parse_env, parse_socket,
    resolve_database_url, resolve_optional_path, resolve_path, set_path, set_string,
    validate_admin_ids, validate_database_parent, validate_private_file, validate_public_url,
    validate_secret, validate_tls_pair, validate_writable_dir,
};
use std::fs;

impl ServerConfig {
    pub(super) fn apply_env_overrides(
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

    pub(super) fn resolve_paths(
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

    pub(super) fn validate(&mut self) -> Result<(), ConfigError> {
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
