use super::{ConfigError, OpenOptions, Path, Write};
use std::fs;

pub(super) fn create_private_dir(path: &Path, field: &'static str) -> Result<(), ConfigError> {
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

pub(super) fn write_new_private_file(
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
