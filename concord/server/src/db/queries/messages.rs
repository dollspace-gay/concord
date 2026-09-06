use sqlx::SqlitePool;

use crate::db::models::MessageRow;

/// Parameters for inserting a channel message.
pub struct InsertMessageParams<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub sender_id: &'a str,
    pub sender_nick: &'a str,
    pub content: &'a str,
    pub reply_to_id: Option<&'a str>,
}

/// Insert a new channel message, optionally replying to another message.
pub async fn insert_message(
    pool: &SqlitePool,
    params: &InsertMessageParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO messages (id, server_id, channel_id, sender_id, sender_nick, content, reply_to_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(params.id)
    .bind(params.server_id)
    .bind(params.channel_id)
    .bind(params.sender_id)
    .bind(params.sender_nick)
    .bind(params.content)
    .bind(params.reply_to_id)
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

/// Get a single message by ID.
pub async fn get_message_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<MessageRow>, sqlx::Error> {
    sqlx::query_as::<_, MessageRow>(
        "SELECT id, server_id, channel_id, sender_id, sender_nick, content, \
         created_at, target_user_id, edited_at, deleted_at, reply_to_id \
         FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Update message content (edit). Sets edited_at to current time.
pub async fn update_message_content(
    pool: &SqlitePool,
    id: &str,
    new_content: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE messages SET content = ?, edited_at = datetime('now') \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(new_content)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Soft-delete a message. Sets deleted_at to current time.
pub async fn soft_delete_message(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE messages SET deleted_at = datetime('now') WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Add a reaction to a message.
pub async fn add_reaction(
    pool: &SqlitePool,
    message_id: &str,
    user_id: &str,
    emoji: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO reactions (message_id, user_id, emoji) VALUES (?, ?, ?)",
    )
    .bind(message_id)
    .bind(user_id)
    .bind(emoji)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Remove a reaction from a message.
pub async fn remove_reaction(
    pool: &SqlitePool,
    message_id: &str,
    user_id: &str,
    emoji: &str,
) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("DELETE FROM reactions WHERE message_id = ? AND user_id = ? AND emoji = ?")
            .bind(message_id)
            .bind(user_id)
            .bind(emoji)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// A reaction record from the database.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReactionRow {
    pub message_id: String,
    pub user_id: String,
    pub emoji: String,
}

/// Get all reactions for a set of message IDs.
pub async fn get_reactions_for_messages(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<Vec<ReactionRow>, sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    // Build a parameterized IN clause
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT message_id, user_id, emoji FROM reactions WHERE message_id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in message_ids {
        separated.push_bind(id);
    }
    builder.push(") ORDER BY created_at");
    builder
        .build_query_as::<ReactionRow>()
        .fetch_all(pool)
        .await
}

/// Upsert a user's read state for a channel.
pub async fn mark_channel_read(
    pool: &SqlitePool,
    user_id: &str,
    channel_id: &str,
    last_read_message_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id, last_read_at) \
         VALUES (?, ?, ?, datetime('now')) \
         ON CONFLICT(user_id, channel_id) DO UPDATE SET \
         last_read_message_id = excluded.last_read_message_id, \
         last_read_at = excluded.last_read_at",
    )
    .bind(user_id)
    .bind(channel_id)
    .bind(last_read_message_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Row for unread count results.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UnreadCountRow {
    pub channel_id: String,
    pub unread_count: i64,
}

/// Get unread message counts for a user across all channels in a server.
/// Counts messages created after the user's last_read_message_id (by created_at).
pub async fn get_unread_counts(
    pool: &SqlitePool,
    user_id: &str,
    server_id: &str,
) -> Result<Vec<UnreadCountRow>, sqlx::Error> {
    sqlx::query_as::<_, UnreadCountRow>(
        "SELECT m.channel_id, COUNT(*) as unread_count \
         FROM messages m \
         LEFT JOIN read_states rs ON rs.user_id = ? AND rs.channel_id = m.channel_id \
         WHERE m.server_id = ? AND m.deleted_at IS NULL \
           AND (rs.last_read_message_id IS NULL OR m.created_at > ( \
             SELECT created_at FROM messages WHERE id = rs.last_read_message_id \
           )) \
         GROUP BY m.channel_id \
         HAVING unread_count > 0",
    )
    .bind(user_id)
    .bind(server_id)
    .fetch_all(pool)
    .await
}

/// Fetch channel message history with cursor-based pagination.
/// Returns messages before `before_time`, ordered newest first.
/// Excludes soft-deleted messages.
pub async fn fetch_channel_history(
    pool: &SqlitePool,
    channel_id: &str,
    before_time: Option<&str>,
    limit: i64,
) -> Result<Vec<MessageRow>, sqlx::Error> {
    match before_time {
        Some(before) => {
            sqlx::query_as::<_, MessageRow>(
                "SELECT id, server_id, channel_id, sender_id, sender_nick, content, \
                 created_at, target_user_id, edited_at, deleted_at, reply_to_id \
                 FROM messages \
                 WHERE channel_id = ? AND created_at < ? AND deleted_at IS NULL \
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
                "SELECT id, server_id, channel_id, sender_id, sender_nick, content, \
                 created_at, target_user_id, edited_at, deleted_at, reply_to_id \
                 FROM messages \
                 WHERE channel_id = ? AND deleted_at IS NULL \
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

/// Get the timestamp of the last message sent by a user in a channel (for slow mode enforcement).
/// Uses `sender_id` (permanent user DID) instead of nickname to prevent bypass via handle changes.
pub async fn get_last_user_message_time(
    pool: &SqlitePool,
    channel_id: &str,
    sender_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let result: Option<(String,)> = sqlx::query_as(
        "SELECT created_at FROM messages \
         WHERE channel_id = ? AND sender_id = ? AND deleted_at IS NULL \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(channel_id)
    .bind(sender_id)
    .fetch_optional(pool)
    .await?;
    Ok(result.map(|r| r.0))
}

#[cfg(test)]
mod tests;
