use sqlx::SqlitePool;

/// Parameters for creating a new OAuth-linked user.
pub struct CreateOAuthUser<'a> {
    pub user_id: &'a str,
    pub username: &'a str,
    pub email: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub oauth_id: &'a str,
    pub provider: &'a str,
    pub provider_id: &'a str,
}

/// Find a user by OAuth provider + provider ID. Returns (user_id, username).
pub async fn find_by_oauth(
    pool: &SqlitePool,
    provider: &str,
    provider_id: &str,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT u.id, u.username FROM users u \
         JOIN oauth_accounts oa ON u.id = oa.user_id \
         WHERE oa.provider = ? AND oa.provider_id = ?",
    )
    .bind(provider)
    .bind(provider_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Create a new user and link an OAuth account.
pub async fn create_with_oauth(
    pool: &SqlitePool,
    params: &CreateOAuthUser<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO users (id, username, email, avatar_url) VALUES (?, ?, ?, ?)")
        .bind(params.user_id)
        .bind(params.username)
        .bind(params.email)
        .bind(params.avatar_url)
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO oauth_accounts (id, user_id, provider, provider_id) VALUES (?, ?, ?, ?)",
    )
    .bind(params.oauth_id)
    .bind(params.user_id)
    .bind(params.provider)
    .bind(params.provider_id)
    .execute(pool)
    .await?;

    // Register primary nickname
    sqlx::query(
        "INSERT OR IGNORE INTO user_nicknames (user_id, nickname, is_primary) VALUES (?, ?, 1)",
    )
    .bind(params.user_id)
    .bind(params.username)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get user by ID. Returns (id, username, email, avatar_url).
pub async fn get_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<(String, String, Option<String>, Option<String>)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT id, username, email, avatar_url FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Store an IRC token hash for a user.
pub async fn create_irc_token(
    pool: &SqlitePool,
    token_id: &str,
    user_id: &str,
    token_hash: &str,
    label: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO irc_tokens (id, user_id, token_hash, label) VALUES (?, ?, ?, ?)")
        .bind(token_id)
        .bind(user_id)
        .bind(token_hash)
        .bind(label)
        .execute(pool)
        .await?;
    Ok(())
}

/// List IRC tokens for a user (id, label, last_used, created_at — NOT the hash).
pub async fn list_irc_tokens(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<(String, Option<String>, Option<String>, String)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, String)>(
        "SELECT id, label, last_used, created_at FROM irc_tokens WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Delete an IRC token by ID (must belong to the user).
pub async fn delete_irc_token(
    pool: &SqlitePool,
    token_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM irc_tokens WHERE id = ? AND user_id = ?")
        .bind(token_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Get all IRC token hashes (for validating IRC PASS). Returns (user_id, username, token_hash).
pub async fn get_all_irc_token_hashes(
    pool: &SqlitePool,
) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT t.user_id, u.username, t.token_hash \
         FROM irc_tokens t JOIN users u ON t.user_id = u.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Get IRC token hashes for a specific nickname (scoped lookup for scalability).
pub async fn get_irc_token_hashes_by_nick(
    pool: &SqlitePool,
    nickname: &str,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT t.user_id, t.token_hash \
         FROM irc_tokens t JOIN users u ON t.user_id = u.id \
         WHERE u.username = ?",
    )
    .bind(nickname)
    .fetch_all(pool)
    .await
}

/// Look up a user profile by nickname, including OAuth provider info.
/// Returns (user_id, username, email, avatar_url, provider, provider_id).
pub async fn get_user_by_nickname(
    pool: &SqlitePool,
    nickname: &str,
) -> Result<
    Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
    sqlx::Error,
> {
    let row = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT u.id, u.username, u.email, u.avatar_url, oa.provider, oa.provider_id \
         FROM users u \
         LEFT JOIN user_nicknames un ON u.id = un.user_id \
         LEFT JOIN oauth_accounts oa ON u.id = oa.user_id \
         WHERE u.username = ? OR un.nickname = ? \
         LIMIT 1",
    )
    .bind(nickname)
    .bind(nickname)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// AT Protocol credentials stored alongside an OAuth account.
pub struct AtprotoCredentials {
    pub did: String,
    pub access_token: String,
    pub refresh_token: String,
    pub dpop_private_key: String,
    pub pds_url: String,
    pub token_expires_at: String,
}

/// Store AT Protocol credentials (tokens, DPoP key, PDS URL) on an oauth_account.
pub async fn store_atproto_credentials(
    pool: &SqlitePool,
    user_id: &str,
    access_token: &str,
    refresh_token: &str,
    dpop_private_key: &str,
    pds_url: &str,
    token_expires_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE oauth_accounts SET access_token = ?, refresh_token = ?, \
         dpop_private_key = ?, pds_url = ?, token_expires_at = ? \
         WHERE user_id = ? AND provider = 'atproto'",
    )
    .bind(access_token)
    .bind(refresh_token)
    .bind(dpop_private_key)
    .bind(pds_url)
    .bind(token_expires_at)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get AT Protocol credentials for a user. Returns None if no atproto account or tokens not stored.
pub async fn get_atproto_credentials(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<AtprotoCredentials>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, String, String, String, String, String)>(
        "SELECT provider_id, access_token, refresh_token, dpop_private_key, pds_url, token_expires_at \
         FROM oauth_accounts \
         WHERE user_id = ? AND provider = 'atproto' \
         AND access_token IS NOT NULL AND dpop_private_key IS NOT NULL",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(did, access_token, refresh_token, dpop_private_key, pds_url, token_expires_at)| {
            AtprotoCredentials {
                did,
                access_token,
                refresh_token,
                dpop_private_key,
                pds_url,
                token_expires_at,
            }
        },
    ))
}

/// Update last_used timestamp for an IRC token.
pub async fn touch_irc_token(
    pool: &SqlitePool,
    user_id: &str,
    token_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE irc_tokens SET last_used = datetime('now') WHERE user_id = ? AND token_hash = ?",
    )
    .bind(user_id)
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
