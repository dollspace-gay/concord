use super::keys::write_private_verified;
use super::*;

#[test]
fn durable_key_copy_is_idempotent_only_for_identical_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let copy = directory.path().join("key-copy");
    write_private_verified(&copy, b"first").unwrap();
    write_private_verified(&copy, b"first").unwrap();
    assert!(write_private_verified(&copy, b"different").is_err());
    assert_eq!(std::fs::read(copy).unwrap(), b"first");
}

#[test]
fn backup_rejects_overlapping_paths() {
    let directory = tempfile::tempdir().unwrap();
    let media = directory.path().join("media");
    std::fs::create_dir(&media).unwrap();
    assert!(reject_overlapping_destination(&media.join("backup"), &[media.as_path()]).is_err());
    assert!(reject_overlapping_destination(directory.path(), &[media.as_path()]).is_err());
}

#[cfg(unix)]
#[test]
fn streaming_copy_preserves_existing_parent_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let parent = directory.path().join("shared");
    std::fs::write(&source, vec![7_u8; 128 * 1024]).unwrap();
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o751)).unwrap();
    copy_private_file(&source, &parent.join("copy")).unwrap();
    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
        0o751
    );
    assert_eq!(
        std::fs::read(parent.join("copy")).unwrap(),
        vec![7_u8; 128 * 1024]
    );
}

#[test]
fn backup_destination_is_claimed_exclusively() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("backup");
    create_private_destination(&destination).unwrap();
    std::fs::write(destination.join("owned-by-first"), b"sentinel").unwrap();
    assert!(create_private_destination(&destination).is_err());
    assert_eq!(
        std::fs::read(destination.join("owned-by-first")).unwrap(),
        b"sentinel"
    );
}
