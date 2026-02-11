use sqlx::SqlitePool;

use crate::db::models::ForumTagRow;

/// Create a new forum tag for a channel.
pub async fn create_tag(
    pool: &SqlitePool,
    id: &str,
    channel_id: &str,
    name: &str,
    emoji: Option<&str>,
    moderated: i32,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO forum_tags (id, channel_id, name, emoji, moderated, position) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(name)
    .bind(emoji)
    .bind(moderated)
    .bind(position)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update an existing forum tag.
pub async fn update_tag(
    pool: &SqlitePool,
    tag_id: &str,
    name: &str,
    emoji: Option<&str>,
    moderated: i32,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE forum_tags SET name = ?, emoji = ?, moderated = ?, position = ? WHERE id = ?",
    )
    .bind(name)
    .bind(emoji)
    .bind(moderated)
    .bind(position)
    .bind(tag_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a forum tag by ID.
pub async fn delete_tag(pool: &SqlitePool, tag_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM forum_tags WHERE id = ?")
        .bind(tag_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List all tags for a channel, ordered by position.
pub async fn list_tags(
    pool: &SqlitePool,
    channel_id: &str,
) -> Result<Vec<ForumTagRow>, sqlx::Error> {
    sqlx::query_as::<_, ForumTagRow>(
        "SELECT * FROM forum_tags WHERE channel_id = ? ORDER BY position",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
}

/// Replace all tags on a thread. Deletes existing associations and inserts new ones.
pub async fn set_thread_tags(
    pool: &SqlitePool,
    thread_id: &str,
    tag_ids: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM thread_tags WHERE thread_id = ?")
        .bind(thread_id)
        .execute(pool)
        .await?;

    for tag_id in tag_ids {
        sqlx::query("INSERT INTO thread_tags (thread_id, tag_id) VALUES (?, ?)")
            .bind(thread_id)
            .bind(tag_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Get all forum tags associated with a thread.
pub async fn get_thread_tags(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<Vec<ForumTagRow>, sqlx::Error> {
    sqlx::query_as::<_, ForumTagRow>(
        "SELECT ft.* FROM forum_tags ft \
         JOIN thread_tags tt ON ft.id = tt.tag_id \
         WHERE tt.thread_id = ? ORDER BY ft.position",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
    use crate::db::queries::messages::{self, InsertMessageParams};
    use crate::db::queries::servers;
    use crate::db::queries::threads;
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_env(pool: &SqlitePool) {
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
        channels::ensure_channel(pool, "c1", "s1", "#forum")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_and_list_tags() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_tag(&pool, "tag1", "c1", "Bug", Some("bug_emoji"), 0, 0)
            .await
            .unwrap();
        create_tag(&pool, "tag2", "c1", "Feature", None, 0, 1)
            .await
            .unwrap();

        let tags = list_tags(&pool, "c1").await.unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "Bug");
        assert_eq!(tags[1].name, "Feature");
    }

    #[tokio::test]
    async fn test_update_tag() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_tag(&pool, "tag1", "c1", "Old", None, 0, 0)
            .await
            .unwrap();

        update_tag(&pool, "tag1", "New", Some("new_emoji"), 1, 5)
            .await
            .unwrap();

        let tags = list_tags(&pool, "c1").await.unwrap();
        assert_eq!(tags[0].name, "New");
        assert_eq!(tags[0].emoji, Some("new_emoji".to_string()));
        assert_eq!(tags[0].moderated, 1);
        assert_eq!(tags[0].position, 5);
    }

    #[tokio::test]
    async fn test_delete_tag() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_tag(&pool, "tag1", "c1", "ToDelete", None, 0, 0)
            .await
            .unwrap();

        delete_tag(&pool, "tag1").await.unwrap();

        let tags = list_tags(&pool, "c1").await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn test_set_and_get_thread_tags() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_tag(&pool, "tag1", "c1", "Bug", None, 0, 0)
            .await
            .unwrap();
        create_tag(&pool, "tag2", "c1", "Help", None, 0, 1)
            .await
            .unwrap();

        // Create a thread
        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Parent",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
        threads::create_thread(
            &pool,
            "t1",
            "s1",
            "Help thread",
            "public_thread",
            "m1",
            1440,
        )
        .await
        .unwrap();

        // Assign tags
        set_thread_tags(&pool, "t1", &["tag1".to_string(), "tag2".to_string()])
            .await
            .unwrap();

        let thread_tags = get_thread_tags(&pool, "t1").await.unwrap();
        assert_eq!(thread_tags.len(), 2);
        assert_eq!(thread_tags[0].name, "Bug");
        assert_eq!(thread_tags[1].name, "Help");
    }

    #[tokio::test]
    async fn test_set_thread_tags_replaces() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_tag(&pool, "tag1", "c1", "Bug", None, 0, 0)
            .await
            .unwrap();
        create_tag(&pool, "tag2", "c1", "Help", None, 0, 1)
            .await
            .unwrap();

        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Parent",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
        threads::create_thread(&pool, "t1", "s1", "Thread", "public_thread", "m1", 60)
            .await
            .unwrap();

        set_thread_tags(&pool, "t1", &["tag1".to_string()])
            .await
            .unwrap();
        assert_eq!(get_thread_tags(&pool, "t1").await.unwrap().len(), 1);

        // Replace with different tags
        set_thread_tags(&pool, "t1", &["tag2".to_string()])
            .await
            .unwrap();
        let tags = get_thread_tags(&pool, "t1").await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "Help");
    }

    #[tokio::test]
    async fn test_list_tags_empty() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let tags = list_tags(&pool, "c1").await.unwrap();
        assert!(tags.is_empty());
    }
}
