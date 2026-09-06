//! Cross-process exclusion for a Concord database.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use sqlx::sqlite::SqliteConnectOptions;

pub fn database_lock_path(database_url: &str) -> Result<PathBuf> {
    let options = SqliteConnectOptions::from_str(database_url)
        .context("database URL is not a valid SQLite URL")?;
    let database = options.get_filename();
    if database == Path::new(":memory:") || database.as_os_str().is_empty() {
        bail!("a persistent database is required for process exclusion");
    }
    let canonical = if database.exists() {
        database.canonicalize()?
    } else {
        let parent = database.parent().context("database path has no parent")?;
        let canonical_parent = if parent.as_os_str().is_empty() {
            std::env::current_dir()?.canonicalize()?
        } else {
            std::fs::create_dir_all(parent)?;
            parent.canonicalize()?
        };
        canonical_parent.join(
            database
                .file_name()
                .context("database path has no file name")?,
        )
    };
    let mut name = canonical
        .file_name()
        .context("database path has no file name")?
        .to_os_string();
    name.push(".concord-maintenance.lock");
    Ok(canonical
        .parent()
        .context("database path has no parent")?
        .join(name))
}

pub fn acquire_database_exclusion(database_url: &str) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(database_lock_path(database_url)?)?;
    FileExt::try_lock_exclusive(&lock)
        .context("another Concord server or maintenance command is active for this database")?;
    Ok(lock)
}

pub fn restore_marker_path(database_url: &str) -> Result<PathBuf> {
    let lock = database_lock_path(database_url)?;
    let name = lock
        .file_name()
        .context("database lock path has no file name")?
        .to_string_lossy();
    let database_name = name
        .strip_suffix(".concord-maintenance.lock")
        .context("database lock path has an unexpected name")?;
    Ok(lock
        .parent()
        .context("database lock path has no parent")?
        .join(format!("{database_name}.concord-restore-pending")))
}

pub fn ensure_restore_is_not_pending(database_url: &str) -> Result<()> {
    let marker = restore_marker_path(database_url)?;
    if marker.exists() {
        bail!(
            "restore activation is incomplete at {}; resume concord-operator backup-restore",
            marker.display()
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn existing_database_symlink_and_real_path_share_lock_identity() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("database.sqlite");
        std::fs::write(&database, b"").unwrap();
        let alias = root.path().join("alias.sqlite");
        symlink(&database, &alias).unwrap();
        let direct = format!("sqlite:{}", database.display());
        let indirect = format!("sqlite:{}", alias.display());
        assert_eq!(
            database_lock_path(&direct).unwrap(),
            database_lock_path(&indirect).unwrap()
        );
        let held = acquire_database_exclusion(&direct).unwrap();
        assert!(acquire_database_exclusion(&indirect).is_err());
        drop(held);
    }

    #[test]
    fn relative_database_without_parent_resolves_beside_current_directory() {
        let expected = std::env::current_dir()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join("concord.db.concord-maintenance.lock");
        assert_eq!(database_lock_path("sqlite:concord.db").unwrap(), expected);
    }

    #[test]
    fn sqlite_url_query_does_not_change_lock_identity() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("query.sqlite");
        let plain = format!("sqlite:{}", database.display());
        let configured = format!("sqlite:{}?mode=rwc&cache=private", database.display());
        assert_eq!(
            database_lock_path(&plain).unwrap(),
            database_lock_path(&configured).unwrap()
        );
    }

    #[test]
    fn concurrent_server_and_operator_handles_are_excluded() {
        let root = tempfile::tempdir().unwrap();
        let database = format!("sqlite:{}", root.path().join("concurrent.sqlite").display());
        let server = acquire_database_exclusion(&database).unwrap();
        let attempted = std::thread::spawn({
            let database = database.clone();
            move || acquire_database_exclusion(&database).is_err()
        })
        .join()
        .unwrap();
        assert!(attempted);
        drop(server);
        assert!(acquire_database_exclusion(&database).is_ok());
    }
}
