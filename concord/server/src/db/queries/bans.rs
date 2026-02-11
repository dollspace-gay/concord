use sqlx::SqlitePool;

use crate::db::models::BanRow;

pub async fn create_ban(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
    user_id: &str,
    banned_by: &str,
    reason: Option<&str>,
    delete_message_days: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bans (id, server_id, user_id, banned_by, reason, delete_message_days) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(server_id)
    .bind(user_id)
    .bind(banned_by)
    .bind(reason)
    .bind(delete_message_days)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_ban(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM bans WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_ban(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Option<BanRow>, sqlx::Error> {
    sqlx::query_as::<_, BanRow>("SELECT * FROM bans WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_bans(pool: &SqlitePool, server_id: &str) -> Result<Vec<BanRow>, sqlx::Error> {
    sqlx::query_as::<_, BanRow>("SELECT * FROM bans WHERE server_id = ? ORDER BY created_at DESC")
        .bind(server_id)
        .fetch_all(pool)
        .await
}

pub async fn is_banned(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bans WHERE server_id = ? AND user_id = ?")
            .bind(server_id)
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(count > 0)
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

    async fn setup_server(pool: &SqlitePool) {
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
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: "u2",
                username: "bob",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u2",
                provider: "github",
                provider_id: "gh-u2",
            },
        )
        .await
        .unwrap();
        servers::create_server(pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_and_get_ban() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_ban(&pool, "b1", "s1", "u2", "u1", Some("Spam"), 0)
            .await
            .unwrap();

        let ban = get_ban(&pool, "s1", "u2").await.unwrap();
        assert!(ban.is_some());
        let b = ban.unwrap();
        assert_eq!(b.user_id, "u2");
        assert_eq!(b.banned_by, "u1");
        assert_eq!(b.reason, Some("Spam".to_string()));
    }

    #[tokio::test]
    async fn test_is_banned() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        assert!(!is_banned(&pool, "s1", "u2").await.unwrap());

        create_ban(&pool, "b1", "s1", "u2", "u1", None, 0)
            .await
            .unwrap();

        assert!(is_banned(&pool, "s1", "u2").await.unwrap());
    }

    #[tokio::test]
    async fn test_list_bans() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        users::create_with_oauth(
            &pool,
            &CreateOAuthUser {
                user_id: "u3",
                username: "charlie",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u3",
                provider: "github",
                provider_id: "gh-u3",
            },
        )
        .await
        .unwrap();

        create_ban(&pool, "b1", "s1", "u2", "u1", None, 0)
            .await
            .unwrap();
        create_ban(&pool, "b2", "s1", "u3", "u1", Some("Abuse"), 7)
            .await
            .unwrap();

        let bans = list_bans(&pool, "s1").await.unwrap();
        assert_eq!(bans.len(), 2);
    }

    #[tokio::test]
    async fn test_remove_ban() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_ban(&pool, "b1", "s1", "u2", "u1", None, 0)
            .await
            .unwrap();

        let removed = remove_ban(&pool, "s1", "u2").await.unwrap();
        assert!(removed);

        assert!(!is_banned(&pool, "s1", "u2").await.unwrap());

        // Removing again returns false
        let removed_again = remove_ban(&pool, "s1", "u2").await.unwrap();
        assert!(!removed_again);
    }

    #[tokio::test]
    async fn test_ban_with_delete_message_days() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_ban(&pool, "b1", "s1", "u2", "u1", None, 7)
            .await
            .unwrap();

        let ban = get_ban(&pool, "s1", "u2").await.unwrap().unwrap();
        assert_eq!(ban.delete_message_days, 7);
    }

    #[tokio::test]
    async fn test_get_nonexistent_ban() {
        let pool = setup_db().await;
        let ban = get_ban(&pool, "nosuch", "nosuch").await.unwrap();
        assert!(ban.is_none());
    }
}
