use super::{
    BackupFile, Digest, OpenOptions, Path, PathBuf, Read, Result, Sha256, SqliteConnectOptions,
    bail, set_private_dir, sync_parent,
};
use anyhow::Context;
use std::fs;
use std::str::FromStr;

pub(super) fn database_path(url: &str) -> Result<PathBuf> {
    let options = SqliteConnectOptions::from_str(url)?;
    let path = options.get_filename();
    if path == Path::new(":memory:") || path.as_os_str().is_empty() {
        bail!("backup/restore requires a persistent SQLite database");
    }
    Ok(path.to_owned())
}

pub(super) fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let created = !destination.exists();
    fs::create_dir_all(destination)?;
    if created {
        set_private_dir(destination)?;
    }
    if !source.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_private_file(&entry.path(), &target)?;
        } else {
            bail!(
                "backup refuses non-file media entry {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn copy_private_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        let created = !parent.exists();
        fs::create_dir_all(parent)?;
        if created {
            set_private_dir(parent)?;
        }
    }
    let mut input = OpenOptions::new()
        .read(true)
        .open(source)
        .with_context(|| format!("read backup source {}", source.display()))?;
    if !input.metadata()?.is_file() {
        bail!("backup source is not a regular file: {}", source.display());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    sync_parent(destination)
}

pub(super) fn reject_overlapping_destination(destination: &Path, sources: &[&Path]) -> Result<()> {
    let destination = canonical_candidate(destination)?;
    for source in sources {
        let source = canonical_candidate(source)?;
        if destination.starts_with(&source) || source.starts_with(&destination) {
            bail!(
                "backup destination overlaps source path {}",
                source.display()
            );
        }
    }
    Ok(())
}

pub(super) fn canonical_candidate(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .context("path has no existing ancestor")?
                .to_owned(),
        );
        ancestor = ancestor.parent().context("path has no existing ancestor")?;
    }
    let mut normalized = ancestor.canonicalize()?;
    for component in suffix.into_iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

pub(super) fn inventory_files(root: &Path) -> Result<Vec<BackupFile>> {
    fn visit(root: &Path, current: &Path, output: &mut Vec<BackupFile>) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), output)?;
            } else if kind.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative != "manifest.json" {
                    output.push(BackupFile {
                        path: relative,
                        size: entry.metadata()?.len(),
                        sha256: sha256_file(&entry.path())?,
                    });
                }
            } else {
                bail!("backup contains unsupported filesystem entry");
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

pub(super) fn checked_backup_path(root: &Path, logical: &str) -> Result<PathBuf> {
    let relative = Path::new(logical);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("unsafe backup manifest path");
    }
    Ok(root.join(relative))
}

pub(super) fn sha256_file(path: &Path) -> Result<String> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}
