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

/// List emoji from all servers a user belongs to where the server allows sharing.
pub async fn list_emoji_for_user_servers(
    pool: &SqlitePool,
    user_id: &str,
    target_server_id: &str,
) -> Result<Vec<EmojiRow>, sqlx::Error> {
    sqlx::query_as::<_, EmojiRow>(
        "SELECT e.id, e.server_id, e.name, e.image_url, e.uploader_id, e.created_at \
         FROM custom_emoji e \
         JOIN servers s ON e.server_id = s.id \
         JOIN server_members sm ON s.id = sm.server_id \
         JOIN servers target ON target.id = ? \
         JOIN server_members target_member ON target_member.server_id = target.id \
             AND target_member.user_id = sm.user_id \
         WHERE sm.user_id = ? AND s.shareable_emoji = 1 \
             AND (e.server_id = target.id OR target.allow_external_emoji = 1) \
         ORDER BY s.name, e.name",
    )
    .bind(target_server_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_emoji(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM custom_emoji WHERE id = ? AND server_id = ?")
        .bind(id)
        .bind(server_id)
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

        let deleted = delete_emoji(&pool, "e1", "s1").await.unwrap();
        assert!(deleted);

        let emojis = list_emoji(&pool, "s1").await.unwrap();
        assert!(emojis.is_empty());

        let deleted_again = delete_emoji(&pool, "e1", "s1").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_list_emoji_empty() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        let emojis = list_emoji(&pool, "s1").await.unwrap();
        assert!(emojis.is_empty());
    }

    #[tokio::test]
    async fn cross_server_emoji_requires_source_sharing_and_target_external_policy() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id,shareable_emoji,allow_external_emoji) VALUES \
             ('source','Source','u1',1,1),('target','Target','u1',1,0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('source','u1','owner'),('target','u1','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        insert_emoji(&pool, "external", "source", "wave", "/wave.png", "u1")
            .await
            .unwrap();

        assert!(
            list_emoji_for_user_servers(&pool, "u1", "target")
                .await
                .unwrap()
                .is_empty()
        );
        sqlx::query("UPDATE servers SET allow_external_emoji=1 WHERE id='target'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            list_emoji_for_user_servers(&pool, "u1", "target")
                .await
                .unwrap()
                .iter()
                .map(|emoji| emoji.id.as_str())
                .collect::<Vec<_>>(),
            vec!["external"]
        );
        sqlx::query("UPDATE servers SET shareable_emoji=0 WHERE id='source'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            list_emoji_for_user_servers(&pool, "u1", "target")
                .await
                .unwrap()
                .is_empty()
        );
    }
}
