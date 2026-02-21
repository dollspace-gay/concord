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
    let placeholders: Vec<&str> = message_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT message_id, user_id, emoji FROM reactions WHERE message_id IN ({}) ORDER BY created_at",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, ReactionRow>(&sql);
    for id in message_ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await
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
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
    use crate::db::queries::servers;
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_server_and_channel(pool: &SqlitePool) {
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

    fn msg_params<'a>(id: &'a str, content: &'a str) -> InsertMessageParams<'a> {
        InsertMessageParams {
            id,
            server_id: "s1",
            channel_id: "c1",
            sender_id: "u1",
            sender_nick: "alice",
            content,
            reply_to_id: None,
        }
    }

    #[tokio::test]
    async fn test_insert_and_get_message() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;

        insert_message(&pool, &msg_params("m1", "Hello world"))
            .await
            .unwrap();

        let msg = get_message_by_id(&pool, "m1").await.unwrap();
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert_eq!(m.id, "m1");
        assert_eq!(m.content, "Hello world");
        assert_eq!(m.sender_nick, "alice");
        assert!(m.edited_at.is_none());
        assert!(m.deleted_at.is_none());
        assert!(m.reply_to_id.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_message() {
        let pool = setup_db().await;
        let msg = get_message_by_id(&pool, "no-such").await.unwrap();
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_edit_message() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;
        insert_message(&pool, &msg_params("m1", "Original"))
            .await
            .unwrap();

        let edited = update_message_content(&pool, "m1", "Edited").await.unwrap();
        assert!(edited);

        let msg = get_message_by_id(&pool, "m1").await.unwrap().unwrap();
        assert_eq!(msg.content, "Edited");
        assert!(msg.edited_at.is_some());
    }

    #[tokio::test]
    async fn test_edit_deleted_message_fails() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;
        insert_message(&pool, &msg_params("m1", "Hello"))
            .await
            .unwrap();

        soft_delete_message(&pool, "m1").await.unwrap();

        // Editing a deleted message should return false
        let edited = update_message_content(&pool, "m1", "New content")
            .await
            .unwrap();
        assert!(!edited);
    }

    #[tokio::test]
    async fn test_soft_delete_message() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;
        insert_message(&pool, &msg_params("m1", "Hello"))
            .await
            .unwrap();

        let deleted = soft_delete_message(&pool, "m1").await.unwrap();
        assert!(deleted);

        let msg = get_message_by_id(&pool, "m1").await.unwrap().unwrap();
        assert!(msg.deleted_at.is_some());

        // Double-delete should return false
        let deleted_again = soft_delete_message(&pool, "m1").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_message_with_reply() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;

        insert_message(&pool, &msg_params("m1", "Hello"))
            .await
            .unwrap();

        let reply_params = InsertMessageParams {
            id: "m2",
            server_id: "s1",
            channel_id: "c1",
            sender_id: "u1",
            sender_nick: "alice",
            content: "Reply!",
            reply_to_id: Some("m1"),
        };
        insert_message(&pool, &reply_params).await.unwrap();

        let reply = get_message_by_id(&pool, "m2").await.unwrap().unwrap();
        assert_eq!(reply.reply_to_id, Some("m1".to_string()));
    }

    #[tokio::test]
    async fn test_fetch_channel_history() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;

        for i in 0..5 {
            insert_message(&pool, &msg_params(&format!("m{i}"), &format!("Msg {i}")))
                .await
                .unwrap();
        }

        // Fetch all (no cursor)
        let history = fetch_channel_history(&pool, "c1", None, 10).await.unwrap();
        assert_eq!(history.len(), 5);
        // Should be newest first
    }

    #[tokio::test]
    async fn test_fetch_channel_history_with_limit() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;

        for i in 0..5 {
            insert_message(&pool, &msg_params(&format!("m{i}"), &format!("Msg {i}")))
                .await
                .unwrap();
        }

        let history = fetch_channel_history(&pool, "c1", None, 2).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_fetch_history_excludes_deleted() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;

        insert_message(&pool, &msg_params("m1", "Keep"))
            .await
            .unwrap();
        insert_message(&pool, &msg_params("m2", "Delete me"))
            .await
            .unwrap();

        soft_delete_message(&pool, "m2").await.unwrap();

        let history = fetch_channel_history(&pool, "c1", None, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Keep");
    }

    #[tokio::test]
    async fn test_reactions() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;
        insert_message(&pool, &msg_params("m1", "Hello"))
            .await
            .unwrap();

        // Add reaction
        let added = add_reaction(&pool, "m1", "u1", "thumbsup").await.unwrap();
        assert!(added);

        // Duplicate reaction should not add
        let dup = add_reaction(&pool, "m1", "u1", "thumbsup").await.unwrap();
        assert!(!dup);

        // Get reactions
        let reactions = get_reactions_for_messages(&pool, &["m1".to_string()])
            .await
            .unwrap();
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].emoji, "thumbsup");

        // Remove reaction
        let removed = remove_reaction(&pool, "m1", "u1", "thumbsup")
            .await
            .unwrap();
        assert!(removed);

        let reactions = get_reactions_for_messages(&pool, &["m1".to_string()])
            .await
            .unwrap();
        assert!(reactions.is_empty());
    }

    #[tokio::test]
    async fn test_reactions_empty_ids() {
        let pool = setup_db().await;
        let reactions = get_reactions_for_messages(&pool, &[]).await.unwrap();
        assert!(reactions.is_empty());
    }

    #[tokio::test]
    async fn test_insert_dm() {
        let pool = setup_db().await;
        users::create_with_oauth(
            &pool,
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
        users::create_with_oauth(
            &pool,
            &CreateOAuthUser {
                user_id: "u2",
                username: "bob",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u2",
                provider: "github",
                provider_id: "gh-u2",
            },
        )
        .await
        .unwrap();

        insert_dm(&pool, "dm1", "u1", "alice", "u2", "Hey Bob!")
            .await
            .unwrap();

        let msg = get_message_by_id(&pool, "dm1").await.unwrap().unwrap();
        assert_eq!(msg.target_user_id, Some("u2".to_string()));
        assert_eq!(msg.content, "Hey Bob!");
        assert!(msg.server_id.is_none());
    }

    #[tokio::test]
    async fn test_mark_channel_read() {
        let pool = setup_db().await;
        setup_server_and_channel(&pool).await;
        insert_message(&pool, &msg_params("m1", "Hello"))
            .await
            .unwrap();

        // Mark as read -- should not error
        mark_channel_read(&pool, "u1", "c1", "m1").await.unwrap();

        // Upsert again with new message
        insert_message(&pool, &msg_params("m2", "World"))
            .await
            .unwrap();
        mark_channel_read(&pool, "u1", "c1", "m2").await.unwrap();
    }
}
