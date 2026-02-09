use sqlx::SqlitePool;

use crate::db::models::{ChannelMemberRow, ChannelRow};

/// Ensure a channel exists in a server, creating it if needed. Returns the channel ID.
pub async fn ensure_channel(
    pool: &SqlitePool,
    channel_id: &str,
    server_id: &str,
    name: &str,
) -> Result<String, sqlx::Error> {
    // Try to find existing channel first
    if let Some(row) = get_channel_by_name(pool, server_id, name).await? {
        return Ok(row.id);
    }
    sqlx::query(
        "INSERT OR IGNORE INTO channels (id, server_id, name) VALUES (?, ?, ?)",
    )
    .bind(channel_id)
    .bind(server_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(channel_id.to_string())
}

/// Get a channel by its UUID.
pub async fn get_channel(
    pool: &SqlitePool,
    channel_id: &str,
) -> Result<Option<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(pool)
        .await
}

/// Get a channel by server_id + name.
pub async fn get_channel_by_name(
    pool: &SqlitePool,
    server_id: &str,
    name: &str,
) -> Result<Option<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT * FROM channels WHERE server_id = ? AND name = ?",
    )
    .bind(server_id)
    .bind(name)
    .fetch_optional(pool)
    .await
}

/// List all channels in a server.
pub async fn list_channels(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT * FROM channels WHERE server_id = ? ORDER BY name",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

/// Get all default channels in a server.
pub async fn get_default_channels(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT * FROM channels WHERE server_id = ? AND is_default = 1 ORDER BY name",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

/// Update a channel's topic.
pub async fn set_topic(
    pool: &SqlitePool,
    channel_id: &str,
    topic: &str,
    set_by: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE channels SET topic = ?, topic_set_by = ?, topic_set_at = datetime('now') WHERE id = ?",
    )
    .bind(topic)
    .bind(set_by)
    .bind(channel_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a channel by ID.
pub async fn delete_channel(pool: &SqlitePool, channel_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Rename a channel.
pub async fn rename_channel(
    pool: &SqlitePool,
    channel_id: &str,
    new_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET name = ? WHERE id = ?")
        .bind(new_name)
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Add a member to a channel.
pub async fn add_member(
    pool: &SqlitePool,
    channel_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO channel_members (channel_id, user_id) VALUES (?, ?)",
    )
    .bind(channel_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove a member from a channel.
pub async fn remove_member(
    pool: &SqlitePool,
    channel_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
        .bind(channel_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get all members of a channel.
pub async fn get_members(
    pool: &SqlitePool,
    channel_id: &str,
) -> Result<Vec<ChannelMemberRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelMemberRow>(
        "SELECT * FROM channel_members WHERE channel_id = ? ORDER BY joined_at",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
}

/// Get all channels a user is a member of within a server.
pub async fn get_user_channels(
    pool: &SqlitePool,
    user_id: &str,
    server_id: &str,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT c.* FROM channels c \
         JOIN channel_members cm ON c.id = cm.channel_id \
         WHERE cm.user_id = ? AND c.server_id = ? \
         ORDER BY c.name",
    )
    .bind(user_id)
    .bind(server_id)
    .fetch_all(pool)
    .await
}
