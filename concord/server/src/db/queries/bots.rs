use sqlx::SqlitePool;

use crate::db::models::BotTokenRow;

pub async fn create_bot_user(
    pool: &SqlitePool,
    user_id: &str,
    username: &str,
    avatar_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (id, username, avatar_url, is_bot)
         VALUES (?, ?, ?, 1)",
    )
    .bind(user_id)
    .bind(username)
    .bind(avatar_url)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_bot_user_owned(
    pool: &SqlitePool,
    user_id: &str,
    username: &str,
    avatar_url: Option<&str>,
    owner_user_id: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query("INSERT INTO users (id, username, avatar_url, is_bot) VALUES (?, ?, ?, 1)")
        .bind(user_id)
        .bind(username)
        .bind(avatar_url)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO bot_ownership(bot_user_id,owner_user_id,repair_required) VALUES (?,?,0)",
    )
    .bind(user_id)
    .bind(owner_user_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn delete_bot_user(pool: &SqlitePool, user_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM users WHERE id=? AND is_bot=1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn record_bot_owner(
    pool: &SqlitePool,
    bot_user_id: &str,
    owner_user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bot_ownership(bot_user_id,owner_user_id,repair_required) VALUES (?,?,0)",
    )
    .bind(bot_user_id)
    .bind(owner_user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn bot_owner(
    pool: &SqlitePool,
    bot_user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT owner_user_id FROM bot_ownership \
         WHERE bot_user_id=? AND repair_required=0",
    )
    .bind(bot_user_id)
    .fetch_optional(pool)
    .await
}

pub async fn bot_token_owner(
    pool: &SqlitePool,
    token_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT o.owner_user_id FROM bot_tokens t \
         JOIN bot_ownership o ON o.bot_user_id=t.user_id \
         WHERE t.id=? AND o.repair_required=0",
    )
    .bind(token_id)
    .fetch_optional(pool)
    .await
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
    let installed_by: String = sqlx::query_scalar("SELECT owner_id FROM servers WHERE id=?")
        .bind(server_id)
        .fetch_optional(pool)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    add_bot_to_server_with_grants(pool, server_id, bot_user_id, &installed_by, "messages").await
}

pub async fn add_bot_to_server_with_grants(
    pool: &SqlitePool,
    server_id: &str,
    bot_user_id: &str,
    installed_by: &str,
    granted_scopes: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let bot: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND is_bot=1 AND disabled_at IS NULL)",
    )
    .bind(bot_user_id)
    .fetch_one(&mut *transaction)
    .await?;
    if !bot {
        return Err(sqlx::Error::RowNotFound);
    }
    sqlx::query(
        "INSERT OR IGNORE INTO server_members (server_id, user_id, role) VALUES (?, ?, 'member')",
    )
    .bind(server_id)
    .bind(bot_user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) \
         VALUES(?,?,?,?,?,'active') ON CONFLICT(bot_user_id,server_id) DO UPDATE SET \
         installed_by=excluded.installed_by,granted_scopes=excluded.granted_scopes,state='active', \
         revoked_at=NULL,authorization_version=bot_installations.authorization_version+1",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(bot_user_id)
    .bind(server_id)
    .bind(installed_by)
    .bind(granted_scopes)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

/// List server IDs that a bot user is a member of.
pub async fn list_bot_server_ids(
    pool: &SqlitePool,
    bot_user_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT server_id FROM server_members WHERE user_id = ?")
            .bind(bot_user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(sid,)| sid).collect())
}

pub async fn remove_bot_from_server(
    pool: &SqlitePool,
    server_id: &str,
    bot_user_id: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "UPDATE bot_installations SET state='revoked',revoked_at=datetime('now'), \
         authorization_version=authorization_version+1 WHERE server_id=? AND bot_user_id=? AND state='active'",
    )
    .bind(server_id)
    .bind(bot_user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(bot_user_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
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

    async fn insert_bot_user(pool: &SqlitePool, user_id: &str, username: &str) {
        create_bot_user(pool, user_id, username, None)
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
        let installed: (String, String, i64) = sqlx::query_as(
            "SELECT state,granted_scopes,authorization_version FROM bot_installations \
             WHERE server_id='s1' AND bot_user_id='bot1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(installed, ("active".into(), "messages".into(), 1));

        remove_bot_from_server(&pool, "s1", "bot1").await.unwrap();

        let member = servers::get_server_member(&pool, "s1", "bot1")
            .await
            .unwrap();
        assert!(member.is_none());
        let revoked: (String, i64) = sqlx::query_as(
            "SELECT state,authorization_version FROM bot_installations WHERE server_id='s1' AND bot_user_id='bot1'",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(revoked, ("revoked".into(), 2));

        add_bot_to_server_with_grants(&pool, "s1", "bot1", "u1", "messages commands")
            .await
            .unwrap();
        let reinstalled: (String, String, i64) = sqlx::query_as(
            "SELECT state,granted_scopes,authorization_version FROM bot_installations WHERE server_id='s1' AND bot_user_id='bot1'",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(
            reinstalled,
            ("active".into(), "messages commands".into(), 3)
        );
    }

    #[tokio::test]
    async fn test_get_nonexistent_bot_token() {
        let pool = setup_db().await;
        let token = get_bot_token_by_hash(&pool, "no-such-hash").await.unwrap();
        assert!(token.is_none());
    }
}
