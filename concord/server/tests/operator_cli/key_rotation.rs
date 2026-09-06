use super::*;

#[test]
fn key_rotation_activates_a_durable_replacement_and_preserves_recovery_key() {
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
    let active = root.join("data/secrets/external-credentials.key");
    let old = fs::read(&active).unwrap();
    let replacement = root.join("replacement.key");
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["key-init", "--key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    let replacement_bytes = fs::read(&replacement).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .args(["secrets-rotate", "--new-key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&active).unwrap(), replacement_bytes);
    let second_replacement = root.join("second-replacement.key");
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["key-init", "--key-file"])
            .arg(&second_replacement)
            .status()
            .unwrap()
            .success()
    );
    let second_bytes = fs::read(&second_replacement).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .args(["secrets-rotate", "--new-key-file"])
            .arg(&second_replacement)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&active).unwrap(), second_bytes);
    let siblings: Vec<_> = fs::read_dir(active.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        siblings
            .iter()
            .any(|name| name.starts_with("external-credentials.previous-"))
    );
    assert!(
        siblings
            .iter()
            .any(|name| name.starts_with("external-credentials.replacement-"))
    );
    assert_ne!(old, replacement_bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "storage-fault-injection")]
fn committed_rotation_recovers_after_sigkill_using_durable_replacement() {
    let fixture = initialized();
    let root = &fixture.root;
    let config = &fixture.config;
    populate_encrypted_rotation_fixture(config, &fixture.binaries.operator);
    let active = root.join("data/secrets/external-credentials.key");
    let replacement = root.join("replacement.key");
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["key-init", "--key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    let replacement_bytes = fs::read(&replacement).unwrap();
    let barrier = root.join("rotation-barrier");
    let mut interrupted = fixture
        .binaries
        .operator
        .command()
        .args(["--config"])
        .arg(config)
        .args(["secrets-rotate", "--new-key-file"])
        .arg(&replacement)
        .env("CONCORD_ROTATION_TEST_BARRIER", &barrier)
        .spawn()
        .unwrap();
    wait_for_or_kill(
        &mut interrupted,
        &std::path::PathBuf::from(format!("{}.database-committed", barrier.display())),
    );
    interrupted.kill().unwrap();
    interrupted.wait().unwrap();
    fs::remove_file(&replacement).unwrap();
    assert!(
        fixture
            .binaries
            .operator
            .command()
            .args(["--config"])
            .arg(config)
            .args(["secrets-rotate", "--new-key-file"])
            .arg(&replacement)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&active).unwrap(), replacement_bytes);

    let loaded = concord_server::config::ServerConfig::load(config).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = concord_server::db::pool::create_pool(&loaded.database.url).await.unwrap();
        let phase: String = sqlx::query_scalar("SELECT phase FROM credential_rotation_state WHERE singleton=1").fetch_one(&pool).await.unwrap();
        assert_eq!(phase, "activated");
        let distinct: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT credential_key_id) FROM oauth_accounts WHERE credential_state='active'").fetch_one(&pool).await.unwrap();
        assert_eq!(distinct, 1);
        let pending_key: String = sqlx::query_scalar("SELECT credential_key_id FROM pending_atproto_oauth WHERE state_hash='pending-state'").fetch_one(&pool).await.unwrap();
        let vault = concord_server::secrets::SecretVault::load(&active).unwrap();
        assert_eq!(pending_key, vault.key_id());
        let credentials = concord_server::db::queries::users::get_atproto_credentials_encrypted(
            &pool, &vault, "at-user",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(credentials.access_token, "access");
        assert_eq!(credentials.refresh_token, "refresh");
        assert_eq!(credentials.dpop_private_key, "dpop");
        let pending_ciphertext: String = sqlx::query_scalar("SELECT credential_ciphertext FROM pending_atproto_oauth WHERE state_hash='pending-state'").fetch_one(&pool).await.unwrap();
        assert_eq!(vault.decrypt("atproto:pending:pending-state", &pending_ciphertext, &pending_key).unwrap(), b"pending-secret");
        let signing: String = sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'").fetch_one(&pool).await.unwrap();
        let signing: serde_json::Value = serde_json::from_str(&signing).unwrap();
        assert_eq!(vault.decrypt("atproto:client-signing-key", signing["ciphertext"].as_str().unwrap(), signing["key_id"].as_str().unwrap()).unwrap(), b"signing-secret");
    });
    fs::remove_dir_all(root).unwrap();
}
