use super::*;
use crate::db::pool::{create_pool, run_migrations};

fn vault(byte: u8) -> (tempfile::NamedTempFile, crate::secrets::SecretVault) {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), hex::encode([byte; 32])).unwrap();
    let vault = crate::secrets::SecretVault::load(file.path()).unwrap();
    (file, vault)
}

#[tokio::test]
async fn signing_key_is_encrypted_stable_and_lost_key_fails() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let (_file, first) = vault(21);
    let generated = AtprotoOAuth::load_or_create(&pool, &first).await.unwrap();
    let expected = serde_json::to_value(&generated.public_jwk).unwrap();
    let stored: String =
        sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored.contains("\"d\":"));
    let loaded = AtprotoOAuth::load_or_create(&pool, &first).await.unwrap();
    assert_eq!(serde_json::to_value(&loaded.public_jwk).unwrap(), expected);
    let (_wrong_file, wrong) = vault(22);
    assert!(AtprotoOAuth::load_or_create(&pool, &wrong).await.is_err());
}

fn pending(state: &str) -> PendingAtprotoAuth {
    let dpop_key = generate_key(KeyType::P256Private).unwrap();
    let dpop_private_key = serde_json::to_string(&jwk::generate(&dpop_key).unwrap()).unwrap();
    let now = Utc::now();
    PendingAtprotoAuth {
        oauth_request: OAuthRequest {
            oauth_state: state.into(),
            issuer: "https://issuer.example".into(),
            authorization_server: "https://issuer.example".into(),
            nonce: "nonce".into(),
            pkce_verifier: "verifier".into(),
            signing_public_key: "public".into(),
            dpop_private_key,
            created_at: now,
            expires_at: now + chrono::Duration::minutes(5),
        },
        dpop_key,
        handle: "alice.example".into(),
        auth_server: AuthorizationServer {
            issuer: "https://issuer.example".into(),
            authorization_endpoint: "https://issuer.example/authorize".into(),
            token_endpoint: "https://issuer.example/token".into(),
            ..Default::default()
        },
        pds_url: "https://pds.example".into(),
        resolved_did: "did:plc:alice".into(),
        created_at: now,
    }
}

#[tokio::test]
async fn pending_oauth_is_encrypted_durable_one_time_and_key_bound() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", directory.path().join("oauth.db").display());
    let pool = create_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let (_file, first) = vault(31);
    store_pending_oauth(&pool, &first, &pending("state-one"))
        .await
        .unwrap();
    let stored: String =
        sqlx::query_scalar("SELECT credential_ciphertext FROM pending_atproto_oauth")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored.contains("verifier"));
    pool.close().await;
    let reopened = create_pool(&url).await.unwrap();
    let recovered = take_pending_oauth(&reopened, &first, "state-one")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.resolved_did, "did:plc:alice");
    assert!(
        take_pending_oauth(&reopened, &first, "state-one")
            .await
            .unwrap()
            .is_none()
    );

    store_pending_oauth(&reopened, &first, &pending("state-two"))
        .await
        .unwrap();
    let (_wrong_file, wrong) = vault(32);
    assert!(
        take_pending_oauth(&reopened, &wrong, "state-two")
            .await
            .is_err()
    );
    assert!(
        take_pending_oauth(&reopened, &first, "state-two")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn corrupt_pending_oauth_fails_closed_and_is_not_replayable() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let (_file, vault) = vault(41);
    store_pending_oauth(&pool, &vault, &pending("state-corrupt"))
        .await
        .unwrap();
    sqlx::query("UPDATE pending_atproto_oauth SET credential_ciphertext='corrupt'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        take_pending_oauth(&pool, &vault, "state-corrupt")
            .await
            .is_err()
    );
    assert!(
        take_pending_oauth(&pool, &vault, "state-corrupt")
            .await
            .unwrap()
            .is_none()
    );
}
