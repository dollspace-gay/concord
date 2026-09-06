use super::*;

#[cfg(unix)]
#[tokio::test]
async fn rooted_media_open_and_delete_reject_parent_symlink_escape() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"outside").unwrap();
    symlink(outside.path(), root.path().join("objects")).unwrap();
    assert!(
        open_rooted_media(root.path(), "objects/secret")
            .await
            .is_err()
    );
    let rooted = open_media_root(root.path()).unwrap();
    assert!(
        rooted_remove(rooted, PathBuf::from("objects/secret"))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(outside.path().join("secret")).unwrap(),
        b"outside"
    );
}
