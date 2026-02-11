use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EmojiRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub image_url: String,
    pub uploader_id: String,
    pub created_at: String,
}

pub async fn list_emoji(pool: &SqlitePool, server_id: &str) -> Result<Vec<EmojiRow>, sqlx::Error> {
    sqlx::query_as::<_, EmojiRow>(
        "SELECT id, server_id, name, image_url, uploader_id, created_at \
         FROM custom_emoji WHERE server_id = ? ORDER BY name",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn get_emoji_by_name(
    pool: &SqlitePool,
    server_id: &str,
    name: &str,
) -> Result<Option<EmojiRow>, sqlx::Error> {
    sqlx::query_as::<_, EmojiRow>(
        "SELECT id, server_id, name, image_url, uploader_id, created_at \
         FROM custom_emoji WHERE server_id = ? AND name = ?",
    )
    .bind(server_id)
    .bind(name)
    .fetch_optional(pool)
    .await
}

pub async fn insert_emoji(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
    name: &str,
    image_url: &str,
    uploader_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO custom_emoji (id, server_id, name, image_url, uploader_id) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(server_id)
    .bind(name)
    .bind(image_url)
    .bind(uploader_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_emoji(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM custom_emoji WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
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
        servers::create_server(pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_insert_and_list_emoji() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        insert_emoji(
            &pool,
            "e1",
            "s1",
            "kappa",
            "https://cdn.example/kappa.png",
            "u1",
        )
        .await
        .unwrap();
        insert_emoji(
            &pool,
            "e2",
            "s1",
            "pogchamp",
            "https://cdn.example/pog.png",
            "u1",
        )
        .await
        .unwrap();

        let emojis = list_emoji(&pool, "s1").await.unwrap();
        assert_eq!(emojis.len(), 2);
        // Ordered by name
        assert_eq!(emojis[0].name, "kappa");
        assert_eq!(emojis[1].name, "pogchamp");
    }

    #[tokio::test]
    async fn test_get_emoji_by_name() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        insert_emoji(
            &pool,
            "e1",
            "s1",
            "kappa",
            "https://cdn.example/kappa.png",
            "u1",
        )
        .await
        .unwrap();

        let emoji = get_emoji_by_name(&pool, "s1", "kappa").await.unwrap();
        assert!(emoji.is_some());
        assert_eq!(emoji.unwrap().image_url, "https://cdn.example/kappa.png");

        let not_found = get_emoji_by_name(&pool, "s1", "nosuch").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_delete_emoji() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        insert_emoji(
            &pool,
            "e1",
            "s1",
            "kappa",
            "https://cdn.example/kappa.png",
            "u1",
        )
        .await
        .unwrap();

        let deleted = delete_emoji(&pool, "e1").await.unwrap();
        assert!(deleted);

        let emojis = list_emoji(&pool, "s1").await.unwrap();
        assert!(emojis.is_empty());

        let deleted_again = delete_emoji(&pool, "e1").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_list_emoji_empty() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        let emojis = list_emoji(&pool, "s1").await.unwrap();
        assert!(emojis.is_empty());
    }
}
