use sqlx::SqlitePool;

use crate::db::models::PinnedMessageRow;

/// Pin a message in a channel. Ignores if already pinned.
pub async fn pin_message(
    pool: &SqlitePool,
    id: &str,
    channel_id: &str,
    message_id: &str,
    pinned_by: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO pinned_messages (id, channel_id, message_id, pinned_by) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(message_id)
    .bind(pinned_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Unpin a message from a channel.
pub async fn unpin_message(
    pool: &SqlitePool,
    channel_id: &str,
    message_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM pinned_messages WHERE channel_id = ? AND message_id = ?")
        .bind(channel_id)
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get all pinned messages in a channel, ordered by most recently pinned first.
pub async fn get_pinned_messages(
    pool: &SqlitePool,
    channel_id: &str,
) -> Result<Vec<PinnedMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, PinnedMessageRow>(
        "SELECT * FROM pinned_messages WHERE channel_id = ? ORDER BY pinned_at DESC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
}

/// Count the number of pinned messages in a channel.
pub async fn count_pins(pool: &SqlitePool, channel_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM pinned_messages WHERE channel_id = ?")
        .bind(channel_id)
        .fetch_one(pool)
        .await
}

/// Check if a specific message is pinned in a channel.
pub async fn is_pinned(
    pool: &SqlitePool,
    channel_id: &str,
    message_id: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pinned_messages WHERE channel_id = ? AND message_id = ?",
    )
    .bind(channel_id)
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
    }

    async fn create_msg(pool: &SqlitePool, id: &str) {
        messages::insert_message(
            pool,
            &InsertMessageParams {
                id,
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Test message",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_pin_and_get_pinned() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_msg(&pool, "m1").await;

        pin_message(&pool, "p1", "c1", "m1", "u1").await.unwrap();

        let pinned = get_pinned_messages(&pool, "c1").await.unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].message_id, "m1");
        assert_eq!(pinned[0].pinned_by, "u1");
    }

    #[tokio::test]
    async fn test_is_pinned() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_msg(&pool, "m1").await;

        assert!(!is_pinned(&pool, "c1", "m1").await.unwrap());

        pin_message(&pool, "p1", "c1", "m1", "u1").await.unwrap();

        assert!(is_pinned(&pool, "c1", "m1").await.unwrap());
    }

    #[tokio::test]
    async fn test_unpin_message() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_msg(&pool, "m1").await;

        pin_message(&pool, "p1", "c1", "m1", "u1").await.unwrap();
        unpin_message(&pool, "c1", "m1").await.unwrap();

        assert!(!is_pinned(&pool, "c1", "m1").await.unwrap());
        let count = count_pins(&pool, "c1").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_count_pins() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        for i in 0..5 {
            let mid = format!("m{i}");
            create_msg(&pool, &mid).await;
            pin_message(&pool, &format!("p{i}"), "c1", &mid, "u1")
                .await
                .unwrap();
        }

        let count = count_pins(&pool, "c1").await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn test_pin_idempotent() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_msg(&pool, "m1").await;

        pin_message(&pool, "p1", "c1", "m1", "u1").await.unwrap();
        // INSERT OR IGNORE -- pinning again should not error or duplicate
        pin_message(&pool, "p2", "c1", "m1", "u1").await.unwrap();

        let count = count_pins(&pool, "c1").await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_empty_pinned_messages() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let pinned = get_pinned_messages(&pool, "c1").await.unwrap();
        assert!(pinned.is_empty());
    }
}
