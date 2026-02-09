use sqlx::SqlitePool;

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

/// Create a new user and link an OAuth account. Returns the user_id.
pub async fn create_with_oauth(
    pool: &SqlitePool,
    user_id: &str,
    username: &str,
    email: Option<&str>,
    avatar_url: Option<&str>,
    oauth_id: &str,
    provider: &str,
    provider_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (id, username, email, avatar_url) VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(username)
    .bind(email)
    .bind(avatar_url)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO oauth_accounts (id, user_id, provider, provider_id) VALUES (?, ?, ?, ?)",
    )
    .bind(oauth_id)
    .bind(user_id)
    .bind(provider)
    .bind(provider_id)
    .execute(pool)
    .await?;

    // Register primary nickname
    sqlx::query(
        "INSERT OR IGNORE INTO user_nicknames (user_id, nickname, is_primary) VALUES (?, ?, 1)",
    )
    .bind(user_id)
    .bind(username)
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
    sqlx::query(
        "INSERT INTO irc_tokens (id, user_id, token_hash, label) VALUES (?, ?, ?, ?)",
    )
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
    let result = sqlx::query(
        "DELETE FROM irc_tokens WHERE id = ? AND user_id = ?",
    )
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
