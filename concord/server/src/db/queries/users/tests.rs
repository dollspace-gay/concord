use super::*;
use crate::db::pool::{create_pool, run_migrations};

async fn setup_db() -> SqlitePool {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

async fn create_test_user(pool: &SqlitePool, id: &str, username: &str) {
    create_with_oauth(
        pool,
        &CreateOAuthUser {
            user_id: id,
            username,
            email: Some("test@example.com"),
            avatar_url: None,
            oauth_id: &format!("oauth-{id}"),
            provider: "github",
            provider_id: &format!("gh-{id}"),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_create_and_get_user() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    let user = get_user(&pool, "u1").await.unwrap();
    assert!(user.is_some());
    let (id, username, email, _avatar) = user.unwrap();
    assert_eq!(id, "u1");
    assert_eq!(username, "alice");
    assert_eq!(email, Some("test@example.com".to_string()));
}

#[tokio::test]
async fn test_find_by_oauth() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    let found = find_by_oauth(&pool, "github", "gh-u1").await.unwrap();
    assert!(found.is_some());
    let (uid, uname) = found.unwrap();
    assert_eq!(uid, "u1");
    assert_eq!(uname, "alice");

    // Non-existent provider/id returns None
    let not_found = find_by_oauth(&pool, "google", "gh-u1").await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_get_user_by_nickname() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    // Should find by username
    let found = get_user_by_nickname(&pool, "alice").await.unwrap();
    assert!(found.is_some());
    let (uid, uname, _email, _avatar, provider, _pid) = found.unwrap();
    assert_eq!(uid, "u1");
    assert_eq!(uname, "alice");
    assert_eq!(provider, Some("github".to_string()));

    // Non-existent nickname returns None
    let not_found = get_user_by_nickname(&pool, "nonexistent").await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_get_nonexistent_user() {
    let pool = setup_db().await;
    let user = get_user(&pool, "no-such-id").await.unwrap();
    assert!(user.is_none());
}

#[tokio::test]
async fn test_irc_token_crud() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    // Create token
    create_irc_token(&pool, "t1", "u1", "hash123", Some("My IRC"))
        .await
        .unwrap();

    // List tokens
    let tokens = list_irc_tokens(&pool, "u1").await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].0, "t1");
    assert_eq!(tokens[0].1, Some("My IRC".to_string()));

    // Get all token hashes
    let all = get_all_irc_token_hashes(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "u1");
    assert_eq!(all[0].1, "alice");
    assert_eq!(all[0].2, "hash123");

    // Touch token
    touch_irc_token(&pool, "u1", "hash123").await.unwrap();
    let tokens_after = list_irc_tokens(&pool, "u1").await.unwrap();
    assert!(tokens_after[0].2.is_some()); // last_used should be set

    // Delete token
    let deleted = delete_irc_token(&pool, "t1", "u1").await.unwrap();
    assert!(deleted);

    let tokens_after_delete = list_irc_tokens(&pool, "u1").await.unwrap();
    assert!(tokens_after_delete.is_empty());
}

#[tokio::test]
async fn test_delete_irc_token_wrong_user() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_test_user(&pool, "u2", "bob").await;

    create_irc_token(&pool, "t1", "u1", "hash123", None)
        .await
        .unwrap();

    // Try to delete u1's token as u2 -- should fail
    let deleted = delete_irc_token(&pool, "t1", "u2").await.unwrap();
    assert!(!deleted);

    // Token should still exist
    let tokens = list_irc_tokens(&pool, "u1").await.unwrap();
    assert_eq!(tokens.len(), 1);
}

#[tokio::test]
async fn test_multiple_irc_tokens() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    create_irc_token(&pool, "t1", "u1", "hash1", Some("Token 1"))
        .await
        .unwrap();
    create_irc_token(&pool, "t2", "u1", "hash2", Some("Token 2"))
        .await
        .unwrap();

    let tokens = list_irc_tokens(&pool, "u1").await.unwrap();
    assert_eq!(tokens.len(), 2);
}

#[tokio::test]
async fn test_atproto_credentials() {
    let pool = setup_db().await;
    // Create user with atproto provider
    create_with_oauth(
        &pool,
        &CreateOAuthUser {
            user_id: "u1",
            username: "alice",
            email: None,
            avatar_url: None,
            oauth_id: "oauth-at1",
            provider: "atproto",
            provider_id: "did:plc:123",
        },
    )
    .await
    .unwrap();

    // Initially no credentials
    let creds = get_atproto_credentials(&pool, "u1").await.unwrap();
    assert!(creds.is_none());

    // Store credentials
    store_atproto_credentials(
        &pool,
        "u1",
        "access-tok",
        "refresh-tok",
        "dpop-key",
        "https://pds.example.com",
        "2026-12-31T00:00:00Z",
    )
    .await
    .unwrap();

    // Retrieve credentials
    let creds = get_atproto_credentials(&pool, "u1").await.unwrap();
    assert!(creds.is_some());
    let c = creds.unwrap();
    assert_eq!(c.did, "did:plc:123");
    assert_eq!(c.access_token, "access-tok");
    assert_eq!(c.pds_url, "https://pds.example.com");
}

#[tokio::test]
async fn encrypted_atproto_credentials_round_trip_and_fail_closed() {
    let pool = setup_db().await;
    create_with_oauth(
        &pool,
        &CreateOAuthUser {
            user_id: "encrypted-user",
            username: "alice",
            email: None,
            avatar_url: None,
            oauth_id: "oauth-encrypted",
            provider: "atproto",
            provider_id: "did:plc:encrypted",
        },
    )
    .await
    .unwrap();
    let first = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(first.path(), hex::encode([11_u8; 32])).unwrap();
    let vault = crate::secrets::SecretVault::load(first.path()).unwrap();
    let expected = AtprotoCredentials {
        did: "did:plc:encrypted".into(),
        access_token: "access-secret".into(),
        refresh_token: "refresh-secret".into(),
        dpop_private_key: "private-jwk".into(),
        pds_url: "https://pds.example.com".into(),
        authorization_issuer: "https://issuer.example.com".into(),
        token_endpoint: "https://issuer.example.com/token".into(),
        token_expires_at: "2026-12-31T00:00:00Z".into(),
        credential_version: 0,
    };
    store_atproto_credentials_encrypted(&pool, &vault, "encrypted-user", &expected)
        .await
        .unwrap();
    let stored:(Option<String>,Option<String>,Option<String>,String)=sqlx::query_as(
        "SELECT access_token,refresh_token,dpop_private_key,credential_state FROM oauth_accounts WHERE user_id='encrypted-user'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(stored, (None, None, None, "active".into()));
    let actual = get_atproto_credentials_encrypted(&pool, &vault, "encrypted-user")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.access_token, expected.access_token);
    assert_eq!(actual.refresh_token, expected.refresh_token);

    let second = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(second.path(), hex::encode([12_u8; 32])).unwrap();
    let wrong = crate::secrets::SecretVault::load(second.path()).unwrap();
    assert!(
        get_atproto_credentials_encrypted(&pool, &wrong, "encrypted-user")
            .await
            .is_err()
    );
    sqlx::query(
        "UPDATE oauth_accounts SET credential_ciphertext='corrupt' WHERE user_id='encrypted-user'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        get_atproto_credentials_encrypted(&pool, &vault, "encrypted-user")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_irc_token_no_label() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    create_irc_token(&pool, "t1", "u1", "hash123", None)
        .await
        .unwrap();

    let tokens = list_irc_tokens(&pool, "u1").await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(tokens[0].1.is_none()); // label should be None
}

#[tokio::test]
async fn test_update_username() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    // Verify initial username
    let user = get_user(&pool, "u1").await.unwrap().unwrap();
    assert_eq!(user.1, "alice");

    // Update username
    update_username(&pool, "u1", "alice.bsky.social")
        .await
        .unwrap();

    // Verify updated username
    let user = get_user(&pool, "u1").await.unwrap().unwrap();
    assert_eq!(user.1, "alice.bsky.social");

    // Verify primary nickname was updated
    let found = get_user_by_nickname(&pool, "alice.bsky.social")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().0, "u1");
}
