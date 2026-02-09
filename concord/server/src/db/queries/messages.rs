use sqlx::SqlitePool;

use crate::db::models::MessageRow;

/// Insert a new channel message.
pub async fn insert_message(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
    channel_id: &str,
    sender_id: &str,
    sender_nick: &str,
    content: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO messages (id, server_id, channel_id, sender_id, sender_nick, content) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(server_id)
    .bind(channel_id)
    .bind(sender_id)
    .bind(sender_nick)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(())
}

/// Insert a direct message.
pub async fn insert_dm(
    pool: &SqlitePool,
    id: &str,
    sender_id: &str,
    sender_nick: &str,
    target_user_id: &str,
    content: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO messages (id, sender_id, sender_nick, target_user_id, content) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(sender_id)
    .bind(sender_nick)
    .bind(target_user_id)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch channel message history with cursor-based pagination.
/// Returns messages before `before_time`, ordered newest first.
pub async fn fetch_channel_history(
    pool: &SqlitePool,
    channel_id: &str,
    before_time: Option<&str>,
    limit: i64,
) -> Result<Vec<MessageRow>, sqlx::Error> {
    match before_time {
        Some(before) => {
            sqlx::query_as::<_, MessageRow>(
                "SELECT id, server_id, channel_id, sender_id, sender_nick, content, created_at, target_user_id \
                 FROM messages \
                 WHERE channel_id = ? AND created_at < ? \
                 ORDER BY created_at DESC \
                 LIMIT ?",
            )
            .bind(channel_id)
            .bind(before)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, MessageRow>(
                "SELECT id, server_id, channel_id, sender_id, sender_nick, content, created_at, target_user_id \
                 FROM messages \
                 WHERE channel_id = ? \
                 ORDER BY created_at DESC \
                 LIMIT ?",
            )
            .bind(channel_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
}
