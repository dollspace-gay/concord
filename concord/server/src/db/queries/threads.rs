use sqlx::SqlitePool;

use crate::db::models::ChannelRow;

/// Create a thread (stored as a channel row with thread-specific fields).
pub async fn create_thread(
    pool: &SqlitePool,
    channel_id: &str,
    server_id: &str,
    name: &str,
    channel_type: &str,
    parent_message_id: &str,
    auto_archive_minutes: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO channels (id, server_id, name, channel_type, thread_parent_message_id, \
         thread_auto_archive_minutes) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(channel_id)
    .bind(server_id)
    .bind(name)
    .bind(channel_type)
    .bind(parent_message_id)
    .bind(auto_archive_minutes)
    .execute(pool)
    .await?;
    Ok(())
}

/// Archive a thread.
pub async fn archive_thread(pool: &SqlitePool, channel_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET archived = 1 WHERE id = ?")
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Unarchive a thread.
pub async fn unarchive_thread(pool: &SqlitePool, channel_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET archived = 0 WHERE id = ?")
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get all threads whose parent message lives in the given channel.
pub async fn get_threads_for_channel(
    pool: &SqlitePool,
    parent_channel_id: &str,
    server_id: &str,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT c.* FROM channels c \
         JOIN messages m ON c.thread_parent_message_id = m.id \
         WHERE m.channel_id = ? AND c.server_id = ? \
         AND c.channel_type IN ('public_thread', 'private_thread')",
    )
    .bind(parent_channel_id)
    .bind(server_id)
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
        // Create a parent message for threads
        messages::insert_message(
            pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Parent message",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_create_thread() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_thread(&pool, "t1", "s1", "Discussion", "public_thread", "m1", 1440)
            .await
            .unwrap();

        let chan = channels::get_channel(&pool, "t1").await.unwrap();
        assert!(chan.is_some());
        let c = chan.unwrap();
        assert_eq!(c.name, "Discussion");
        assert_eq!(c.channel_type, "public_thread");
        assert_eq!(c.thread_parent_message_id, Some("m1".to_string()));
        assert_eq!(c.thread_auto_archive_minutes, 1440);
        assert_eq!(c.archived, 0);
    }

    #[tokio::test]
    async fn test_archive_and_unarchive_thread() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_thread(&pool, "t1", "s1", "Thread", "public_thread", "m1", 60)
            .await
            .unwrap();

        archive_thread(&pool, "t1").await.unwrap();
        let chan = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(chan.archived, 1);

        unarchive_thread(&pool, "t1").await.unwrap();
        let chan = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(chan.archived, 0);
    }

    #[tokio::test]
    async fn test_get_threads_for_channel() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_thread(&pool, "t1", "s1", "Thread 1", "public_thread", "m1", 60)
            .await
            .unwrap();
        create_thread(&pool, "t2", "s1", "Thread 2", "private_thread", "m1", 1440)
            .await
            .unwrap();

        let threads = get_threads_for_channel(&pool, "c1", "s1").await.unwrap();
        assert_eq!(threads.len(), 2);
    }

    #[tokio::test]
    async fn test_no_threads_for_channel() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let threads = get_threads_for_channel(&pool, "c1", "s1").await.unwrap();
        assert!(threads.is_empty());
    }

    #[tokio::test]
    async fn test_private_thread() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_thread(
            &pool,
            "t1",
            "s1",
            "Secret Thread",
            "private_thread",
            "m1",
            60,
        )
        .await
        .unwrap();

        let chan = channels::get_channel(&pool, "t1").await.unwrap().unwrap();
        assert_eq!(chan.channel_type, "private_thread");
    }
}
