use sqlx::SqlitePool;

/// Remove a member from a server (kick -- they can rejoin).
pub async fn kick_member(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    // Remove from server_members
    let result = sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    // Also remove their role assignments
    let _ = sqlx::query("DELETE FROM user_roles WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await;
    Ok(result.rows_affected() > 0)
}

/// Set timeout on a member. Pass None to clear timeout.
pub async fn set_member_timeout(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    timeout_until: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE server_members SET timeout_until = ? WHERE server_id = ? AND user_id = ?",
    )
    .bind(timeout_until)
    .bind(server_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Get the timeout_until value for a member.
pub async fn get_member_timeout(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let result: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT timeout_until FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(result.and_then(|r| r.0))
}

/// Set slow mode seconds on a channel.
pub async fn set_slowmode(
    pool: &SqlitePool,
    channel_id: &str,
    seconds: i32,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE channels SET slowmode_seconds = ? WHERE id = ?")
        .bind(seconds)
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Set NSFW flag on a channel.
pub async fn set_nsfw(
    pool: &SqlitePool,
    channel_id: &str,
    is_nsfw: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE channels SET is_nsfw = ? WHERE id = ?")
        .bind(is_nsfw as i32)
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Bulk delete messages by IDs (soft delete). Returns number of messages deleted.
pub async fn bulk_delete_messages(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<u64, sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(0);
    }
    // Build parameterized IN clause
    let placeholders: Vec<&str> = message_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "UPDATE messages SET deleted_at = datetime('now') WHERE id IN ({}) AND deleted_at IS NULL",
        placeholders.join(",")
    );
    let mut query = sqlx::query(&sql);
    for id in message_ids {
        query = query.bind(id);
    }
    let result = query.execute(pool).await?;
    Ok(result.rows_affected())
}

/// Delete messages from a user in a server within the last N days (for ban purge).
pub async fn delete_user_messages(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    days: i32,
) -> Result<u64, sqlx::Error> {
    if days <= 0 {
        return Ok(0);
    }
    let result = sqlx::query(
        "UPDATE messages SET deleted_at = datetime('now') WHERE server_id = ? AND sender_id = ? AND deleted_at IS NULL AND created_at >= datetime('now', ?)",
    )
    .bind(server_id)
    .bind(user_id)
    .bind(format!("-{days} days"))
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
    use crate::db::queries::messages::{self, InsertMessageParams};
    use crate::db::queries::roles::{self, CreateRoleParams};
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
        users::create_with_oauth(
            pool,
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
        servers::create_server(pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
        servers::add_server_member(pool, "s1", "u2", "member")
            .await
            .unwrap();
        channels::ensure_channel(pool, "c1", "s1", "#general")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_kick_member() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        // Assign a role to verify it gets removed too
        roles::create_role(
            &pool,
            &CreateRoleParams {
                id: "r1",
                server_id: "s1",
                name: "Mod",
                color: None,
                icon_url: None,
                position: 1,
                permissions: 0,
                is_default: false,
            },
        )
        .await
        .unwrap();
        roles::assign_role(&pool, "s1", "u2", "r1").await.unwrap();

        let kicked = kick_member(&pool, "s1", "u2").await.unwrap();
        assert!(kicked);

        // Member should be gone
        let member = servers::get_server_member(&pool, "s1", "u2").await.unwrap();
        assert!(member.is_none());

        // Role assignments should be gone too
        let user_roles = roles::get_user_roles(&pool, "s1", "u2").await.unwrap();
        assert!(user_roles.is_empty());
    }

    #[tokio::test]
    async fn test_kick_nonexistent_member() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let kicked = kick_member(&pool, "s1", "nosuch").await.unwrap();
        assert!(!kicked);
    }

    #[tokio::test]
    async fn test_set_and_get_member_timeout() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        // Initially no timeout
        let timeout = get_member_timeout(&pool, "s1", "u2").await.unwrap();
        assert!(timeout.is_none());

        // Set timeout
        let set = set_member_timeout(&pool, "s1", "u2", Some("2027-01-01T00:00:00Z"))
            .await
            .unwrap();
        assert!(set);

        let timeout = get_member_timeout(&pool, "s1", "u2").await.unwrap();
        assert_eq!(timeout, Some("2027-01-01T00:00:00Z".to_string()));

        // Clear timeout
        set_member_timeout(&pool, "s1", "u2", None).await.unwrap();
        let timeout = get_member_timeout(&pool, "s1", "u2").await.unwrap();
        assert!(timeout.is_none());
    }

    #[tokio::test]
    async fn test_set_slowmode() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let set = set_slowmode(&pool, "c1", 30).await.unwrap();
        assert!(set);

        let chan = channels::get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.slowmode_seconds, 30);

        // Disable slowmode
        set_slowmode(&pool, "c1", 0).await.unwrap();
        let chan = channels::get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.slowmode_seconds, 0);
    }

    #[tokio::test]
    async fn test_set_nsfw() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let set = set_nsfw(&pool, "c1", true).await.unwrap();
        assert!(set);

        let chan = channels::get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.is_nsfw, 1);

        set_nsfw(&pool, "c1", false).await.unwrap();
        let chan = channels::get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.is_nsfw, 0);
    }

    #[tokio::test]
    async fn test_bulk_delete_messages() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        for i in 0..5 {
            messages::insert_message(
                &pool,
                &InsertMessageParams {
                    id: &format!("m{i}"),
                    server_id: "s1",
                    channel_id: "c1",
                    sender_id: "u1",
                    sender_nick: "alice",
                    content: &format!("Message {i}"),
                    reply_to_id: None,
                },
            )
            .await
            .unwrap();
        }

        let deleted = bulk_delete_messages(
            &pool,
            &["m0".to_string(), "m1".to_string(), "m2".to_string()],
        )
        .await
        .unwrap();
        assert_eq!(deleted, 3);

        // Verify they are soft-deleted
        let m0 = messages::get_message_by_id(&pool, "m0")
            .await
            .unwrap()
            .unwrap();
        assert!(m0.deleted_at.is_some());

        // Remaining messages should be intact
        let m3 = messages::get_message_by_id(&pool, "m3")
            .await
            .unwrap()
            .unwrap();
        assert!(m3.deleted_at.is_none());
    }

    #[tokio::test]
    async fn test_bulk_delete_empty() {
        let pool = setup_db().await;
        let deleted = bulk_delete_messages(&pool, &[]).await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_delete_user_messages() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u2",
                sender_nick: "bob",
                content: "Bob's message",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        let deleted = delete_user_messages(&pool, "s1", "u2", 7).await.unwrap();
        assert_eq!(deleted, 1);

        let m1 = messages::get_message_by_id(&pool, "m1")
            .await
            .unwrap()
            .unwrap();
        assert!(m1.deleted_at.is_some());
    }

    #[tokio::test]
    async fn test_delete_user_messages_zero_days() {
        let pool = setup_db().await;
        let deleted = delete_user_messages(&pool, "s1", "u2", 0).await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_set_slowmode_nonexistent_channel() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        let set = set_slowmode(&pool, "nonexistent", 10).await.unwrap();
        assert!(!set);
    }
}
