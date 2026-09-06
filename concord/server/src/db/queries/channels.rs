use sqlx::SqlitePool;

use crate::db::models::{ChannelMemberRow, ChannelRow};

/// Atomically create a channel and its initial private visibility grant.
pub struct CreateScopedChannel<'a> {
    pub channel_id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub category_id: Option<&'a str>,
    pub is_private: bool,
    pub channel_type: &'a str,
    pub creator_user_id: &'a str,
}

pub async fn create_scoped_channel(
    pool: &SqlitePool,
    params: &CreateScopedChannel<'_>,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO channels(id,server_id,name,category_id,is_private,channel_type) \
         SELECT ?,?,?,?,?,? WHERE ? IS NULL OR EXISTS( \
            SELECT 1 FROM channel_categories WHERE id=? AND server_id=? \
         )",
    )
    .bind(params.channel_id)
    .bind(params.server_id)
    .bind(params.name)
    .bind(params.category_id)
    .bind(params.is_private as i32)
    .bind(params.channel_type)
    .bind(params.category_id)
    .bind(params.category_id)
    .bind(params.server_id)
    .execute(&mut *transaction)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(sqlx::Error::Protocol(
            "category does not belong to channel server".into(),
        ));
    }
    if params.is_private {
        sqlx::query("INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) VALUES(?,'user',?)")
            .bind(params.channel_id).bind(params.creator_user_id).execute(&mut *transaction).await?;
    }
    sqlx::query("UPDATE servers SET authorization_version=authorization_version+1 WHERE id=?")
        .bind(params.server_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await
}

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
    sqlx::query("INSERT OR IGNORE INTO channels (id, server_id, name) VALUES (?, ?, ?)")
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
    sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels WHERE server_id = ? AND name = ?")
        .bind(server_id)
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// List all channels in a server, ordered by position then name.
pub async fn list_channels(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT * FROM channels WHERE server_id = ? ORDER BY position, name",
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
    sqlx::query("INSERT OR IGNORE INTO channel_members (channel_id, user_id) VALUES (?, ?)")
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

/// Update a channel's position.
pub async fn update_channel_position(
    pool: &SqlitePool,
    channel_id: &str,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET position = ? WHERE id = ?")
        .bind(position)
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update a channel's category.
pub async fn update_channel_category(
    pool: &SqlitePool,
    channel_id: &str,
    category_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET category_id = ? WHERE id = ?")
        .bind(category_id)
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set a channel's private flag.
pub async fn set_channel_private(
    pool: &SqlitePool,
    channel_id: &str,
    is_private: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET is_private = ? WHERE id = ?")
        .bind(is_private as i32)
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get channel permission overrides.
pub async fn get_channel_overrides(
    pool: &SqlitePool,
    channel_id: &str,
) -> Result<Vec<crate::db::models::ChannelPermissionOverrideRow>, sqlx::Error> {
    sqlx::query_as::<_, crate::db::models::ChannelPermissionOverrideRow>(
        "SELECT * FROM channel_permission_overrides WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
}

/// Set (upsert) a channel permission override.
pub async fn set_channel_override(
    pool: &SqlitePool,
    id: &str,
    channel_id: &str,
    target_type: &str,
    target_id: &str,
    allow_bits: i64,
    deny_bits: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO channel_permission_overrides \
         (id, channel_id, target_type, target_id, allow_bits, deny_bits) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(channel_id, target_type, target_id) DO UPDATE SET \
         allow_bits = excluded.allow_bits, deny_bits = excluded.deny_bits",
    )
    .bind(id)
    .bind(channel_id)
    .bind(target_type)
    .bind(target_id)
    .bind(allow_bits)
    .bind(deny_bits)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a channel permission override.
pub async fn delete_channel_override(
    pool: &SqlitePool,
    channel_id: &str,
    target_type: &str,
    target_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM channel_permission_overrides \
         WHERE channel_id = ? AND target_type = ? AND target_id = ?",
    )
    .bind(channel_id)
    .bind(target_type)
    .bind(target_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Check if a user is a member of a specific channel (for private channel access).
pub async fn is_channel_member(
    pool: &SqlitePool,
    channel_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_members WHERE channel_id = ? AND user_id = ?",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests;
