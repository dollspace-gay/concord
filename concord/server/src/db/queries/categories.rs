use sqlx::SqlitePool;

use crate::db::models::ChannelCategoryRow;

/// Create a new channel category in a server.
pub async fn create_category(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
    name: &str,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO channel_categories (id, server_id, name, position) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(server_id)
    .bind(name)
    .bind(position)
    .execute(pool)
    .await?;
    Ok(())
}

/// List all categories in a server, ordered by position.
pub async fn list_categories(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ChannelCategoryRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelCategoryRow>(
        "SELECT * FROM channel_categories WHERE server_id = ? ORDER BY position",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

/// Update a category's name.
pub async fn update_category(
    pool: &SqlitePool,
    category_id: &str,
    name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channel_categories SET name = ? WHERE id = ?")
        .bind(name)
        .bind(category_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update a category's position.
pub async fn update_category_position(
    pool: &SqlitePool,
    category_id: &str,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channel_categories SET position = ? WHERE id = ?")
        .bind(position)
        .bind(category_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a category. Channels in this category will have category_id set to NULL.
pub async fn delete_category(pool: &SqlitePool, category_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM channel_categories WHERE id = ?")
        .bind(category_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get a category by ID.
pub async fn get_category(
    pool: &SqlitePool,
    category_id: &str,
) -> Result<Option<ChannelCategoryRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelCategoryRow>("SELECT * FROM channel_categories WHERE id = ?")
        .bind(category_id)
        .fetch_optional(pool)
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
    async fn test_create_and_get_category() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_category(&pool, "cat1", "s1", "Text Channels", 0)
            .await
            .unwrap();

        let cat = get_category(&pool, "cat1").await.unwrap();
        assert!(cat.is_some());
        let c = cat.unwrap();
        assert_eq!(c.name, "Text Channels");
        assert_eq!(c.position, 0);
    }

    #[tokio::test]
    async fn test_list_categories_ordered_by_position() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_category(&pool, "cat1", "s1", "Voice", 2)
            .await
            .unwrap();
        create_category(&pool, "cat2", "s1", "Text", 1)
            .await
            .unwrap();
        create_category(&pool, "cat3", "s1", "Info", 0)
            .await
            .unwrap();

        let cats = list_categories(&pool, "s1").await.unwrap();
        assert_eq!(cats.len(), 3);
        assert_eq!(cats[0].name, "Info");
        assert_eq!(cats[1].name, "Text");
        assert_eq!(cats[2].name, "Voice");
    }

    #[tokio::test]
    async fn test_update_category_name() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_category(&pool, "cat1", "s1", "Old Name", 0)
            .await
            .unwrap();

        update_category(&pool, "cat1", "New Name").await.unwrap();

        let cat = get_category(&pool, "cat1").await.unwrap().unwrap();
        assert_eq!(cat.name, "New Name");
    }

    #[tokio::test]
    async fn test_update_category_position() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_category(&pool, "cat1", "s1", "Cat", 0)
            .await
            .unwrap();

        update_category_position(&pool, "cat1", 5).await.unwrap();

        let cat = get_category(&pool, "cat1").await.unwrap().unwrap();
        assert_eq!(cat.position, 5);
    }

    #[tokio::test]
    async fn test_delete_category() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_category(&pool, "cat1", "s1", "ToDelete", 0)
            .await
            .unwrap();

        delete_category(&pool, "cat1").await.unwrap();

        let cat = get_category(&pool, "cat1").await.unwrap();
        assert!(cat.is_none());
    }

    #[tokio::test]
    async fn test_list_categories_empty_server() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        let cats = list_categories(&pool, "s1").await.unwrap();
        assert!(cats.is_empty());
    }
}
