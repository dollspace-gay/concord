use sqlx::SqlitePool;

use crate::db::models::BotTokenRow;

pub async fn create_bot_user(
    pool: &SqlitePool,
    user_id: &str,
    username: &str,
    avatar_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (id, username, avatar_url, provider, provider_id, is_bot)
         VALUES (?, ?, ?, 'bot', ?, 1)",
    )
    .bind(user_id)
    .bind(username)
    .bind(avatar_url)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_bot_token(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    token_hash: &str,
    name: &str,
    scopes: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bot_tokens (id, user_id, token_hash, name, scopes) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(user_id)
    .bind(token_hash)
    .bind(name)
    .bind(scopes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_bot_token_by_hash(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<BotTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, BotTokenRow>("SELECT * FROM bot_tokens WHERE token_hash = ?")
        .bind(token_hash)
        .fetch_optional(pool)
        .await
}

/// Fetch all bot tokens for iterate-and-verify authentication.
/// Argon2 hashes include a random salt, so we must verify the raw token
/// against each stored hash rather than hashing and looking up.
pub async fn get_all_bot_tokens(pool: &SqlitePool) -> Result<Vec<BotTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, BotTokenRow>("SELECT * FROM bot_tokens")
        .fetch_all(pool)
        .await
}

pub async fn list_bot_tokens(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<BotTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, BotTokenRow>(
        "SELECT * FROM bot_tokens WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_bot_token(pool: &SqlitePool, token_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM bot_tokens WHERE id = ?")
        .bind(token_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_token_last_used(pool: &SqlitePool, token_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE bot_tokens SET last_used = datetime('now') WHERE id = ?")
        .bind(token_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn is_bot_user(pool: &SqlitePool, user_id: &str) -> Result<bool, sqlx::Error> {
    let row: Option<(i32,)> = sqlx::query_as("SELECT is_bot FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some_and(|(v,)| v == 1))
}

pub async fn add_bot_to_server(
    pool: &SqlitePool,
    server_id: &str,
    bot_user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO server_members (server_id, user_id, role) VALUES (?, ?, 'member')",
    )
    .bind(server_id)
    .bind(bot_user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_bot_from_server(
    pool: &SqlitePool,
    server_id: &str,
    bot_user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(bot_user_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::servers;
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_owner(pool: &SqlitePool) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: "u1",
                username: "alice",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u1",
                provider: "github",
                provider_id: "gh-u1",
            },
        )
        .await
        .unwrap();
    }

    /// Create a bot user directly in the users table (the create_bot_user function
    /// references columns that don't exist in the current schema, so we insert manually).
    async fn insert_bot_user(pool: &SqlitePool, user_id: &str, username: &str) {
        sqlx::query("INSERT INTO users (id, username, is_bot) VALUES (?, ?, 1)")
            .bind(user_id)
            .bind(username)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_bot_token_and_lookup() {
        let pool = setup_db().await;
        insert_bot_user(&pool, "bot1", "TestBot").await;

        let is = is_bot_user(&pool, "bot1").await.unwrap();
        assert!(is);

        create_bot_token(&pool, "bt1", "bot1", "hash123", "Default", "messages.read")
            .await
            .unwrap();

        let token = get_bot_token_by_hash(&pool, "hash123").await.unwrap();
        assert!(token.is_some());
        let t = token.unwrap();
        assert_eq!(t.user_id, "bot1");
        assert_eq!(t.name, "Default");
        assert_eq!(t.scopes, "messages.read");
    }

    #[tokio::test]
    async fn test_list_bot_tokens() {
        let pool = setup_db().await;
        insert_bot_user(&pool, "bot1", "TestBot").await;

        create_bot_token(&pool, "bt1", "bot1", "hash1", "Token1", "read")
            .await
            .unwrap();
        create_bot_token(&pool, "bt2", "bot1", "hash2", "Token2", "write")
            .await
            .unwrap();

        let tokens = list_bot_tokens(&pool, "bot1").await.unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_bot_token() {
        let pool = setup_db().await;
        insert_bot_user(&pool, "bot1", "TestBot").await;
        create_bot_token(&pool, "bt1", "bot1", "hash1", "Token1", "read")
            .await
            .unwrap();

        delete_bot_token(&pool, "bt1").await.unwrap();

        let token = get_bot_token_by_hash(&pool, "hash1").await.unwrap();
        assert!(token.is_none());
    }

    #[tokio::test]
    async fn test_update_token_last_used() {
        let pool = setup_db().await;
        insert_bot_user(&pool, "bot1", "TestBot").await;
        create_bot_token(&pool, "bt1", "bot1", "hash1", "Token1", "read")
            .await
            .unwrap();

        // Initially last_used is None
        let t = get_bot_token_by_hash(&pool, "hash1")
            .await
            .unwrap()
            .unwrap();
        assert!(t.last_used.is_none());

        update_token_last_used(&pool, "bt1").await.unwrap();

        let t = get_bot_token_by_hash(&pool, "hash1")
            .await
            .unwrap()
            .unwrap();
        assert!(t.last_used.is_some());
    }

    #[tokio::test]
    async fn test_is_bot_user_false_for_regular_user() {
        let pool = setup_db().await;
        setup_owner(&pool).await;

        let is_bot = is_bot_user(&pool, "u1").await.unwrap();
        assert!(!is_bot);
    }

    #[tokio::test]
    async fn test_add_and_remove_bot_from_server() {
        let pool = setup_db().await;
        setup_owner(&pool).await;
        servers::create_server(&pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
        insert_bot_user(&pool, "bot1", "TestBot").await;

        add_bot_to_server(&pool, "s1", "bot1").await.unwrap();

        let member = servers::get_server_member(&pool, "s1", "bot1")
            .await
            .unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().role, "member");

        remove_bot_from_server(&pool, "s1", "bot1").await.unwrap();

        let member = servers::get_server_member(&pool, "s1", "bot1")
            .await
            .unwrap();
        assert!(member.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_bot_token() {
        let pool = setup_db().await;
        let token = get_bot_token_by_hash(&pool, "no-such-hash").await.unwrap();
        assert!(token.is_none());
    }
}
