use sqlx::SqlitePool;

use crate::db::models::{NotificationSettingRow, UpsertNotificationParams};

/// Upsert a notification setting for a user (server-level or channel-level).
///
/// SQLite's UNIQUE constraint does not enforce uniqueness when any column is NULL
/// (because NULL != NULL). For server-level settings (channel_id = NULL), we use a
/// DELETE + INSERT pattern to prevent duplicate rows. For channel-level settings,
/// ON CONFLICT works normally.
pub async fn upsert_notification_setting(
    pool: &SqlitePool,
    params: &UpsertNotificationParams<'_>,
) -> Result<(), sqlx::Error> {
    if params.channel_id.is_none() {
        // Server-level setting: DELETE existing row first to avoid duplicates
        sqlx::query(
            "DELETE FROM notification_settings \
             WHERE user_id = ? AND server_id = ? AND channel_id IS NULL",
        )
        .bind(params.user_id)
        .bind(params.server_id)
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT INTO notification_settings (id, user_id, server_id, channel_id, level, \
             suppress_everyone, suppress_roles, muted, mute_until, updated_at) \
             VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, datetime('now'))",
        )
        .bind(params.id)
        .bind(params.user_id)
        .bind(params.server_id)
        .bind(params.level)
        .bind(params.suppress_everyone as i32)
        .bind(params.suppress_roles as i32)
        .bind(params.muted as i32)
        .bind(params.mute_until)
        .execute(pool)
        .await?;
    } else {
        // Channel-level setting: ON CONFLICT works because channel_id is non-NULL
        sqlx::query(
            "INSERT INTO notification_settings (id, user_id, server_id, channel_id, level, \
             suppress_everyone, suppress_roles, muted, mute_until, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now')) \
             ON CONFLICT(user_id, server_id, channel_id) DO UPDATE SET \
             level = excluded.level, suppress_everyone = excluded.suppress_everyone, \
             suppress_roles = excluded.suppress_roles, muted = excluded.muted, \
             mute_until = excluded.mute_until, updated_at = datetime('now')",
        )
        .bind(params.id)
        .bind(params.user_id)
        .bind(params.server_id)
        .bind(params.channel_id)
        .bind(params.level)
        .bind(params.suppress_everyone as i32)
        .bind(params.suppress_roles as i32)
        .bind(params.muted as i32)
        .bind(params.mute_until)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Get notification settings for a user in a server.
pub async fn get_notification_settings(
    pool: &SqlitePool,
    user_id: &str,
    server_id: &str,
) -> Result<Vec<NotificationSettingRow>, sqlx::Error> {
    sqlx::query_as::<_, NotificationSettingRow>(
        "SELECT id, user_id, server_id, channel_id, level, suppress_everyone, \
         suppress_roles, muted, mute_until, created_at, updated_at \
         FROM notification_settings WHERE user_id = ? AND (server_id = ? OR server_id IS NULL) \
         ORDER BY channel_id NULLS FIRST",
    )
    .bind(user_id)
    .bind(server_id)
    .fetch_all(pool)
    .await
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

    async fn setup_server(pool: &SqlitePool) {
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
    }

    #[tokio::test]
    async fn test_upsert_and_get_server_level_settings() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns1",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: None,
                level: "mentions",
                suppress_everyone: true,
                suppress_roles: false,
                muted: false,
                mute_until: None,
            },
        )
        .await
        .unwrap();

        let settings = get_notification_settings(&pool, "u1", "s1").await.unwrap();
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].level, "mentions");
        assert_eq!(settings[0].suppress_everyone, 1);
        assert_eq!(settings[0].suppress_roles, 0);
        assert!(settings[0].channel_id.is_none());
    }

    #[tokio::test]
    async fn test_upsert_server_level_no_duplicates() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        // Insert server-level setting (channel_id = NULL)
        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns1",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: None,
                level: "all",
                suppress_everyone: false,
                suppress_roles: false,
                muted: false,
                mute_until: None,
            },
        )
        .await
        .unwrap();

        // Upsert again with different values — should replace, not duplicate
        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns2",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: None,
                level: "none",
                suppress_everyone: true,
                suppress_roles: true,
                muted: true,
                mute_until: None,
            },
        )
        .await
        .unwrap();

        let settings = get_notification_settings(&pool, "u1", "s1").await.unwrap();
        assert_eq!(
            settings.len(),
            1,
            "Should have exactly 1 server-level row, not duplicates"
        );
        assert_eq!(settings[0].level, "none");
        assert_eq!(settings[0].suppress_everyone, 1);
        assert_eq!(settings[0].muted, 1);
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        crate::db::queries::channels::ensure_channel(&pool, "c1", "s1", "#general")
            .await
            .unwrap();

        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns1",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: Some("c1"),
                level: "all",
                suppress_everyone: false,
                suppress_roles: false,
                muted: false,
                mute_until: None,
            },
        )
        .await
        .unwrap();

        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns2",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: Some("c1"),
                level: "none",
                suppress_everyone: true,
                suppress_roles: true,
                muted: true,
                mute_until: Some("2027-01-01T00:00:00Z"),
            },
        )
        .await
        .unwrap();

        let settings = get_notification_settings(&pool, "u1", "s1").await.unwrap();
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].level, "none");
        assert_eq!(settings[0].muted, 1);
    }

    #[tokio::test]
    async fn test_channel_level_settings() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        crate::db::queries::channels::ensure_channel(&pool, "c1", "s1", "#general")
            .await
            .unwrap();

        // Server-level setting
        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns1",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: None,
                level: "all",
                suppress_everyone: false,
                suppress_roles: false,
                muted: false,
                mute_until: None,
            },
        )
        .await
        .unwrap();

        // Channel-level setting
        upsert_notification_setting(
            &pool,
            &UpsertNotificationParams {
                id: "ns2",
                user_id: "u1",
                server_id: Some("s1"),
                channel_id: Some("c1"),
                level: "mentions",
                suppress_everyone: false,
                suppress_roles: false,
                muted: true,
                mute_until: None,
            },
        )
        .await
        .unwrap();

        let settings = get_notification_settings(&pool, "u1", "s1").await.unwrap();
        assert_eq!(settings.len(), 2);
        // Server-level (channel_id NULL) should come first
        assert!(settings[0].channel_id.is_none());
        assert!(settings[1].channel_id.is_some());
    }

    #[tokio::test]
    async fn test_empty_settings() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        let settings = get_notification_settings(&pool, "u1", "s1").await.unwrap();
        assert!(settings.is_empty());
    }
}
