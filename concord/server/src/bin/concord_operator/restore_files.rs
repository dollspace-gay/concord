use super::{OpenOptions, Path, PathBuf, RestoreMarker, Result, Uuid, bail, write_private_new};
use anyhow::Context;
use std::fs;

pub(super) fn ensure_empty_restore_destination(
    database: &Path,
    media: &Path,
    key: &Path,
) -> Result<()> {
    if database.exists() {
        bail!("restore database destination must not exist");
    }
    for suffix in ["-wal", "-shm"] {
        if PathBuf::from(format!("{}{suffix}", database.display())).exists() {
            bail!("restore database sidecar destination must not exist");
        }
    }
    if media.exists() && fs::read_dir(media)?.next().is_some() {
        bail!("restore media destination must be empty");
    }
    if key.exists() {
        bail!("restore external credential key destination must not exist");
    }
    Ok(())
}

pub(super) fn staged_path(destination: &Path, backup_id: &str) -> Result<PathBuf> {
    let name = destination
        .file_name()
        .context("restore destination has no file name")?
        .to_string_lossy();
    Ok(destination
        .parent()
        .context("restore destination has no parent")?
        .join(format!(".{name}.restore-{backup_id}")))
}

pub(super) fn activate_staged_path(staged: &Path, destination: &Path) -> Result<()> {
    match (staged.exists(), destination.exists()) {
        (true, false) => {
            fs::rename(staged, destination)?;
            sync_parent(destination)
        }
        (false, true) => Ok(()),
        (true, true) => bail!("restore activation found both staged and destination paths"),
        (false, false) => bail!("restore activation is missing staged and destination paths"),
    }
}

pub(super) fn activate_staged_tree(staged: &Path, destination: &Path) -> Result<()> {
    if !staged.exists() {
        if destination.is_dir() {
            return Ok(());
        }
        bail!("restore activation is missing staged and destination media paths");
    }
    if !destination.exists() {
        fs::rename(staged, destination)?;
        return sync_parent(destination);
    }
    if !staged.is_dir() || !destination.is_dir() {
        bail!("restore media activation encountered a non-directory path");
    }
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() && target.is_dir() {
            activate_staged_tree(&entry.path(), &target)?;
        } else if !target.exists() {
            fs::rename(entry.path(), &target)?;
            sync_parent(&target)?;
        } else {
            bail!("restore media activation found a destination collision");
        }
    }
    fs::remove_dir(staged)?;
    sync_parent(staged)
}

pub(super) fn remove_staging_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) fn write_restore_marker(path: &Path, marker: &RestoreMarker) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    write_private_new(&temporary, &serde_json::to_vec(marker)?)?;
    fs::rename(&temporary, path)?;
    sync_parent(path)
}

pub(super) fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}
