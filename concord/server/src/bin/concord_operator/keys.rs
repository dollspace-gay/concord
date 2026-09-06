use super::{OpenOptions, Path, PathBuf, Result, Rng, Write};
use anyhow::Context;
use std::fs;

pub(super) fn set_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(super) fn create_private_destination(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path).context("create exclusive backup destination")?;
    set_private_dir(path)
}

pub(super) fn write_new_key(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut random = [0_u8; 32];
    rand::rng().fill_bytes(&mut random);
    write_private_new(path, hex::encode(random).as_bytes())
}

pub(super) fn backup_current_key(path: &Path, key_id: &str) -> Result<PathBuf> {
    let backup = path.with_extension(format!("previous-{key_id}"));
    let bytes = std::fs::read(path)?;
    write_private_verified(&backup, &bytes)?;
    Ok(backup)
}

pub(super) fn backup_replacement_key(
    source: &Path,
    active: &Path,
    key_id: &str,
) -> Result<PathBuf> {
    let durable = active.with_extension(format!("replacement-{key_id}"));
    write_private_verified(&durable, &std::fs::read(source)?)?;
    Ok(durable)
}

pub(super) fn activate_key(source: &Path, destination: &Path) -> Result<()> {
    let bytes = std::fs::read(source)?;
    let temporary = destination.with_extension(format!("activate-{}", uuid::Uuid::new_v4()));
    write_private_new(&temporary, &bytes)?;
    std::fs::rename(&temporary, destination)?;
    if let Some(parent) = destination.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

pub(super) fn write_private_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Some(parent) = path.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

pub(super) fn write_private_verified(path: &Path, bytes: &[u8]) -> Result<()> {
    match write_private_new(path, bytes) {
        Ok(()) => Ok(()),
        Err(error) if path.exists() => {
            let existing = std::fs::read(path)?;
            if existing == bytes {
                OpenOptions::new().read(true).open(path)?.sync_all()?;
                if let Some(parent) = path.parent() {
                    OpenOptions::new().read(true).open(parent)?.sync_all()?;
                }
                Ok(())
            } else {
                Err(error).with_context(|| {
                    format!("existing durable key copy {} differs", path.display())
                })
            }
        }
        Err(error) => Err(error),
    }
}
