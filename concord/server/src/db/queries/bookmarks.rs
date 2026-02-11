use sqlx::SqlitePool;

use crate::db::models::BookmarkRow;

/// Add a bookmark on a message for a user. Ignores if already bookmarked.
pub async fn add_bookmark(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    message_id: &str,
    note: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO bookmarks (id, user_id, message_id, note) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(user_id)
    .bind(message_id)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove a bookmark for a user on a specific message.
pub async fn remove_bookmark(
    pool: &SqlitePool,
    user_id: &str,
    message_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM bookmarks WHERE user_id = ? AND message_id = ?")
        .bind(user_id)
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List all bookmarks for a user, ordered by most recently created first.
pub async fn list_bookmarks(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<BookmarkRow>, sqlx::Error> {
    sqlx::query_as::<_, BookmarkRow>(
        "SELECT * FROM bookmarks WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Check if a user has bookmarked a specific message.
pub async fn is_bookmarked(
    pool: &SqlitePool,
    user_id: &str,
    message_id: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE user_id = ? AND message_id = ?")
            .bind(user_id)
            .bind(message_id)
            .fetch_one(pool)
            .await?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
    use crate::db::queries::messages::{self, InsertMessageParams};
    use crate::db::queries::servers;
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
        channels::ensure_channel(pool, "c1", "s1", "#general")
            .await
            .unwrap();
        messages::insert_message(
            pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Bookmark me",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_add_and_list_bookmarks() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        add_bookmark(&pool, "bk1", "u1", "m1", Some("Important"))
            .await
            .unwrap();

        let bookmarks = list_bookmarks(&pool, "u1").await.unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].message_id, "m1");
        assert_eq!(bookmarks[0].note, Some("Important".to_string()));
    }

    #[tokio::test]
    async fn test_is_bookmarked() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        assert!(!is_bookmarked(&pool, "u1", "m1").await.unwrap());

        add_bookmark(&pool, "bk1", "u1", "m1", None).await.unwrap();

        assert!(is_bookmarked(&pool, "u1", "m1").await.unwrap());
    }

    #[tokio::test]
    async fn test_remove_bookmark() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        add_bookmark(&pool, "bk1", "u1", "m1", None).await.unwrap();

        remove_bookmark(&pool, "u1", "m1").await.unwrap();

        assert!(!is_bookmarked(&pool, "u1", "m1").await.unwrap());
        let bookmarks = list_bookmarks(&pool, "u1").await.unwrap();
        assert!(bookmarks.is_empty());
    }

    #[tokio::test]
    async fn test_bookmark_idempotent() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        add_bookmark(&pool, "bk1", "u1", "m1", None).await.unwrap();
        // INSERT OR IGNORE -- second add should not error
        add_bookmark(&pool, "bk2", "u1", "m1", Some("Note"))
            .await
            .unwrap();

        let bookmarks = list_bookmarks(&pool, "u1").await.unwrap();
        assert_eq!(bookmarks.len(), 1);
    }

    #[tokio::test]
    async fn test_bookmark_no_note() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        add_bookmark(&pool, "bk1", "u1", "m1", None).await.unwrap();

        let bookmarks = list_bookmarks(&pool, "u1").await.unwrap();
        assert!(bookmarks[0].note.is_none());
    }

    #[tokio::test]
    async fn test_empty_bookmarks() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let bookmarks = list_bookmarks(&pool, "u1").await.unwrap();
        assert!(bookmarks.is_empty());
    }
}
