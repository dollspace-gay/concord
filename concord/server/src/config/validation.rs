use super::{
    ConfigError, MIN_SECRET_BYTES, OpenOptions, Path, PathBuf, SAMPLE_SECRETS, SocketAddr, Url,
    fmt, invalid,
};
use std::fs;

pub fn is_loopback_public_url(value: &str) -> bool {
    Url::parse(value)
        .is_ok_and(|url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
}

pub(super) fn validate_public_url(value: &str) -> Result<String, ConfigError> {
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

pub(super) fn validate_secret(secret: &str) -> Result<(), ConfigError> {
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

pub(super) fn validate_tls_pair(
    cert: &Option<PathBuf>,
    key: &Option<PathBuf>,
) -> Result<(), ConfigError> {
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

pub(super) fn validate_admin_ids(ids: &[String]) -> Result<(), ConfigError> {
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

pub(super) fn validate_database_parent(url: &str) -> Result<(), ConfigError> {
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

pub(super) fn validate_writable_dir(path: &Path, field: &'static str) -> Result<(), ConfigError> {
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

pub(super) fn validate_private_file(path: &Path, field: &'static str) -> Result<(), ConfigError> {
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

pub(super) fn validate_readable_file(path: &Path, field: &'static str) -> Result<(), ConfigError> {
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

pub(super) fn parse_socket(field: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        field,
        reason: "must be an IP socket address such as 127.0.0.1:8080".into(),
    })
}

pub(super) fn bounded<T: Copy + PartialOrd + fmt::Display>(
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
