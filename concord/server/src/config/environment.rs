use super::{ConfigError, Path, PathBuf};

pub(super) fn parse_env<T: std::str::FromStr>(
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

pub(super) fn set_string(
    environment: &impl Fn(&str) -> Option<String>,
    name: &str,
    target: &mut String,
) {
    if let Some(value) = environment(name) {
        *target = value;
    }
}

pub(super) fn set_path(
    environment: &impl Fn(&str) -> Option<String>,
    name: &str,
    target: &mut Option<PathBuf>,
) {
    if let Some(value) = environment(name) {
        *target = Some(value.into());
    }
}

pub(super) fn resolve_optional_path(root: &Path, path: &mut Option<PathBuf>) {
    if let Some(path) = path {
        resolve_path(root, path);
    }
}

pub(super) fn resolve_path(root: &Path, path: &mut PathBuf) {
    if path.is_relative() {
        *path = root.join(&*path);
    }
}

pub(super) fn resolve_database_url(root: &Path, value: &mut String) {
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

pub(super) fn invalid<T>(field: &'static str, reason: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid {
        field,
        reason: reason.into(),
    })
}
