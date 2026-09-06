use super::{Arc, MediaError, Path, PathBuf, safe_storage_key};

pub(super) fn open_media_root(path: &Path) -> Result<Arc<cap_std::fs::Dir>, MediaError> {
    Ok(Arc::new(cap_std::fs::Dir::open_ambient_dir(
        path,
        cap_std::ambient_authority(),
    )?))
}

pub(super) async fn rooted_create_dir_all(
    root: Arc<cap_std::fs::Dir>,
    path: PathBuf,
) -> Result<(), MediaError> {
    tokio::task::spawn_blocking(move || root.create_dir_all(path))
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(())
}

pub(super) async fn rooted_open_new(
    root: Arc<cap_std::fs::Dir>,
    path: PathBuf,
) -> Result<tokio::fs::File, MediaError> {
    let file = tokio::task::spawn_blocking(move || {
        root.open_with(
            path,
            cap_std::fs::OpenOptions::new().create_new(true).write(true),
        )
    })
    .await
    .map_err(|_| MediaError::Invalid)??;
    Ok(tokio::fs::File::from_std(file.into_std()))
}

pub async fn open_rooted_media(path: &Path, key: &str) -> Result<tokio::fs::File, MediaError> {
    if !safe_storage_key(key) {
        return Err(MediaError::Invalid);
    }
    let root = open_media_root(path)?;
    let key = PathBuf::from(key);
    let file = tokio::task::spawn_blocking(move || root.open(key))
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(tokio::fs::File::from_std(file.into_std()))
}

pub(super) async fn rooted_remove(
    root: Arc<cap_std::fs::Dir>,
    path: PathBuf,
) -> Result<(), std::io::Error> {
    tokio::task::spawn_blocking(move || root.remove_file(path))
        .await
        .map_err(|_| std::io::Error::other("media filesystem task failed"))?
}

pub(super) async fn rooted_rename(
    root: Arc<cap_std::fs::Dir>,
    from: PathBuf,
    to: PathBuf,
) -> Result<(), MediaError> {
    let target = root.clone();
    tokio::task::spawn_blocking(move || root.rename(from, &target, to))
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(())
}

pub(super) async fn rooted_sync_dir(
    root: Arc<cap_std::fs::Dir>,
    path: PathBuf,
) -> Result<(), MediaError> {
    tokio::task::spawn_blocking(move || root.open(path)?.into_std().sync_all())
        .await
        .map_err(|_| MediaError::Invalid)??;
    Ok(())
}
