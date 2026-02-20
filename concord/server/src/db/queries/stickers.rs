use sqlx::SqlitePool;

use crate::db::models::StickerRow;

pub async fn list_stickers(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<StickerRow>, sqlx::Error> {
    sqlx::query_as::<_, StickerRow>(
        "SELECT id, server_id, name, image_url, description, uploader_id, created_at \
         FROM stickers WHERE server_id = ? ORDER BY name",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn get_sticker_by_name(
    pool: &SqlitePool,
    server_id: &str,
    name: &str,
) -> Result<Option<StickerRow>, sqlx::Error> {
    sqlx::query_as::<_, StickerRow>(
        "SELECT id, server_id, name, image_url, description, uploader_id, created_at \
         FROM stickers WHERE server_id = ? AND name = ?",
    )
    .bind(server_id)
    .bind(name)
    .fetch_optional(pool)
    .await
}

pub async fn insert_sticker(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
    name: &str,
    image_url: &str,
    description: Option<&str>,
    uploader_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO stickers (id, server_id, name, image_url, description, uploader_id) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(server_id)
    .bind(name)
    .bind(image_url)
    .bind(description)
    .bind(uploader_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_sticker(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM stickers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// List stickers from all servers a user belongs to where the server allows sharing.
pub async fn list_stickers_for_user_servers(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<StickerRow>, sqlx::Error> {
    sqlx::query_as::<_, StickerRow>(
        "SELECT st.id, st.server_id, st.name, st.image_url, st.description, st.uploader_id, st.created_at \
         FROM stickers st \
         JOIN servers s ON st.server_id = s.id \
         JOIN server_members sm ON s.id = sm.server_id \
         WHERE sm.user_id = ? AND s.shareable_emoji = 1 \
         ORDER BY s.name, st.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
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
    async fn test_insert_and_list_stickers() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        insert_sticker(
            &pool,
            "st1",
            "s1",
            "wave",
            "https://cdn.example/wave.png",
            Some("A wave"),
            "u1",
        )
        .await
        .unwrap();
        insert_sticker(
            &pool,
            "st2",
            "s1",
            "dance",
            "https://cdn.example/dance.png",
            None,
            "u1",
        )
        .await
        .unwrap();

        let stickers = list_stickers(&pool, "s1").await.unwrap();
        assert_eq!(stickers.len(), 2);
        assert_eq!(stickers[0].name, "dance");
        assert_eq!(stickers[1].name, "wave");
    }

    #[tokio::test]
    async fn test_get_sticker_by_name() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        insert_sticker(
            &pool,
            "st1",
            "s1",
            "wave",
            "https://cdn.example/wave.png",
            None,
            "u1",
        )
        .await
        .unwrap();

        let sticker = get_sticker_by_name(&pool, "s1", "wave").await.unwrap();
        assert!(sticker.is_some());
        assert_eq!(sticker.unwrap().image_url, "https://cdn.example/wave.png");

        let not_found = get_sticker_by_name(&pool, "s1", "nosuch").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_delete_sticker() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        insert_sticker(
            &pool,
            "st1",
            "s1",
            "wave",
            "https://cdn.example/wave.png",
            None,
            "u1",
        )
        .await
        .unwrap();

        let deleted = delete_sticker(&pool, "st1").await.unwrap();
        assert!(deleted);

        let stickers = list_stickers(&pool, "s1").await.unwrap();
        assert!(stickers.is_empty());

        let deleted_again = delete_sticker(&pool, "st1").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_list_stickers_empty() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        let stickers = list_stickers(&pool, "s1").await.unwrap();
        assert!(stickers.is_empty());
    }
}
