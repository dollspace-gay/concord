use super::*;

#[test]
fn media_inventory_remains_available_when_provider_key_is_lost_and_honors_exclusion() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("secrets-migrate")
            .status()
            .unwrap()
            .success()
    );
    fs::remove_file(root.join("data/secrets/external-credentials.key")).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .arg("media-inventory")
            .status()
            .unwrap()
            .success()
    );

    let lock_path = root.join("data/concord.db.concord-maintenance.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    FileExt::try_lock_exclusive(&lock).unwrap();
    let blocked = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .arg("media-inventory")
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("maintenance"));
    fs::remove_dir_all(root).unwrap();
}
