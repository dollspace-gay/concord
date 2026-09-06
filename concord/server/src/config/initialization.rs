use super::{
    AuthConfig, ConfigError, Path, Rng, ServerConfig, create_private_dir, write_new_private_file,
};

impl ServerConfig {
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
        rand::rng().fill_bytes(&mut random);
        let secret = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        write_new_private_file(&secret_path, secret.as_bytes(), "auth.jwt_secret_file")?;
        rand::rng().fill_bytes(&mut random);
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
}
