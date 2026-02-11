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

    async fn create_test_user(pool: &SqlitePool, id: &str, username: &str) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: id,
                username,
                email: None,
                avatar_url: None,
                oauth_id: &format!("oauth-{id}"),
                provider: "github",
                provider_id: &format!("gh-{id}"),
            },
        )
        .await
        .unwrap();
    }

    async fn create_test_server(pool: &SqlitePool, sid: &str, uid: &str) {
        servers::create_server(pool, sid, "Test Server", uid, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_ensure_channel_creates_new() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;

        let id = ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();
        assert_eq!(id, "c1");

        let chan = get_channel(&pool, "c1").await.unwrap();
        assert!(chan.is_some());
        assert_eq!(chan.unwrap().name, "#general");
    }

    #[tokio::test]
    async fn test_ensure_channel_idempotent() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;

        let id1 = ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();
        // Calling again with different proposed id should return the existing one
        let id2 = ensure_channel(&pool, "c2", "s1", "#general").await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn test_get_channel_by_name() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#random").await.unwrap();

        let chan = get_channel_by_name(&pool, "s1", "#random").await.unwrap();
        assert!(chan.is_some());
        assert_eq!(chan.unwrap().id, "c1");

        let not_found = get_channel_by_name(&pool, "s1", "#nosuch").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_list_channels() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;

        ensure_channel(&pool, "c1", "s1", "#alpha").await.unwrap();
        ensure_channel(&pool, "c2", "s1", "#beta").await.unwrap();

        let channels = list_channels(&pool, "s1").await.unwrap();
        assert_eq!(channels.len(), 2);
    }

    #[tokio::test]
    async fn test_set_topic() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        set_topic(&pool, "c1", "Welcome!", "alice").await.unwrap();

        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.topic, "Welcome!");
        assert_eq!(chan.topic_set_by, Some("alice".to_string()));
        assert!(chan.topic_set_at.is_some());
    }

    #[tokio::test]
    async fn test_delete_channel() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        delete_channel(&pool, "c1").await.unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap();
        assert!(chan.is_none());
    }

    #[tokio::test]
    async fn test_rename_channel() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#old-name")
            .await
            .unwrap();

        rename_channel(&pool, "c1", "#new-name").await.unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.name, "#new-name");
    }

    #[tokio::test]
    async fn test_channel_members() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_user(&pool, "u2", "bob").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        // Add members
        add_member(&pool, "c1", "u1").await.unwrap();
        add_member(&pool, "c1", "u2").await.unwrap();

        assert!(is_channel_member(&pool, "c1", "u1").await.unwrap());
        assert!(is_channel_member(&pool, "c1", "u2").await.unwrap());

        let members = get_members(&pool, "c1").await.unwrap();
        assert_eq!(members.len(), 2);

        // Remove member
        remove_member(&pool, "c1", "u2").await.unwrap();
        assert!(!is_channel_member(&pool, "c1", "u2").await.unwrap());
    }

    #[tokio::test]
    async fn test_add_member_idempotent() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        add_member(&pool, "c1", "u1").await.unwrap();
        add_member(&pool, "c1", "u1").await.unwrap(); // Should not error

        let members = get_members(&pool, "c1").await.unwrap();
        assert_eq!(members.len(), 1);
    }

    #[tokio::test]
    async fn test_update_channel_position() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        update_channel_position(&pool, "c1", 5).await.unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.position, 5);
    }

    #[tokio::test]
    async fn test_update_channel_category() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        // Create a real category first (FK constraint)
        crate::db::queries::categories::create_category(&pool, "cat1", "s1", "Text", 0)
            .await
            .unwrap();

        update_channel_category(&pool, "c1", Some("cat1"))
            .await
            .unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.category_id, Some("cat1".to_string()));

        update_channel_category(&pool, "c1", None).await.unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert!(chan.category_id.is_none());
    }

    #[tokio::test]
    async fn test_set_channel_private() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#secret").await.unwrap();

        set_channel_private(&pool, "c1", true).await.unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.is_private, 1);

        set_channel_private(&pool, "c1", false).await.unwrap();
        let chan = get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.is_private, 0);
    }

    #[tokio::test]
    async fn test_channel_permission_overrides() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();

        // Set an override
        set_channel_override(&pool, "o1", "c1", "role", "r1", 0x1, 0x2)
            .await
            .unwrap();

        let overrides = get_channel_overrides(&pool, "c1").await.unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].target_type, "role");
        assert_eq!(overrides[0].target_id, "r1");
        assert_eq!(overrides[0].allow_bits, 0x1);
        assert_eq!(overrides[0].deny_bits, 0x2);

        // Upsert same override with new values
        set_channel_override(&pool, "o2", "c1", "role", "r1", 0x3, 0x4)
            .await
            .unwrap();
        let overrides = get_channel_overrides(&pool, "c1").await.unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].allow_bits, 0x3);

        // Delete override
        delete_channel_override(&pool, "c1", "role", "r1")
            .await
            .unwrap();
        let overrides = get_channel_overrides(&pool, "c1").await.unwrap();
        assert!(overrides.is_empty());
    }

    #[tokio::test]
    async fn test_get_user_channels() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_server(&pool, "s1", "u1").await;
        ensure_channel(&pool, "c1", "s1", "#general").await.unwrap();
        ensure_channel(&pool, "c2", "s1", "#random").await.unwrap();

        add_member(&pool, "c1", "u1").await.unwrap();

        let user_channels = get_user_channels(&pool, "u1", "s1").await.unwrap();
        assert_eq!(user_channels.len(), 1);
        assert_eq!(user_channels[0].name, "#general");
    }
}
