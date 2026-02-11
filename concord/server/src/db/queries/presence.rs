use sqlx::SqlitePool;

use crate::db::models::UserPresenceRow;

/// Upsert a user's presence status.
pub async fn upsert_presence(
    pool: &SqlitePool,
    user_id: &str,
    status: &str,
    custom_status: Option<&str>,
    status_emoji: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_presence (user_id, status, custom_status, status_emoji, last_seen_at, updated_at) \
         VALUES (?, ?, ?, ?, datetime('now'), datetime('now')) \
         ON CONFLICT(user_id) DO UPDATE SET status = excluded.status, \
         custom_status = excluded.custom_status, status_emoji = excluded.status_emoji, \
         last_seen_at = datetime('now'), updated_at = datetime('now')",
    )
    .bind(user_id)
    .bind(status)
    .bind(custom_status)
    .bind(status_emoji)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a single user's presence.
pub async fn get_presence(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<UserPresenceRow>, sqlx::Error> {
    sqlx::query_as::<_, UserPresenceRow>(
        "SELECT user_id, status, custom_status, status_emoji, last_seen_at, updated_at \
         FROM user_presence WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Get presence for multiple users (e.g., all members of a server).
pub async fn get_presences_for_users(
    pool: &SqlitePool,
    user_ids: &[String],
) -> Result<Vec<UserPresenceRow>, sqlx::Error> {
    if user_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<&str> = user_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT user_id, status, custom_status, status_emoji, last_seen_at, updated_at \
         FROM user_presence WHERE user_id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, UserPresenceRow>(&sql);
    for id in user_ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await
}

/// Set user offline and record last_seen.
pub async fn set_offline(pool: &SqlitePool, user_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_presence SET status = 'offline', last_seen_at = datetime('now'), \
         updated_at = datetime('now') WHERE user_id = ?",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn create_user(pool: &SqlitePool, id: &str) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: id,
                username: &format!("user-{id}"),
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

    #[tokio::test]
    async fn test_upsert_and_get_presence() {
        let pool = setup_db().await;
        create_user(&pool, "u1").await;

        upsert_presence(&pool, "u1", "online", None, None)
            .await
            .unwrap();

        let p = get_presence(&pool, "u1").await.unwrap();
        assert!(p.is_some());
        let pres = p.unwrap();
        assert_eq!(pres.status, "online");
        assert!(pres.custom_status.is_none());
    }

    #[tokio::test]
    async fn test_upsert_with_custom_status() {
        let pool = setup_db().await;
        create_user(&pool, "u1").await;

        upsert_presence(&pool, "u1", "dnd", Some("Busy coding"), Some("laptop"))
            .await
            .unwrap();

        let pres = get_presence(&pool, "u1").await.unwrap().unwrap();
        assert_eq!(pres.status, "dnd");
        assert_eq!(pres.custom_status, Some("Busy coding".to_string()));
        assert_eq!(pres.status_emoji, Some("laptop".to_string()));
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let pool = setup_db().await;
        create_user(&pool, "u1").await;

        upsert_presence(&pool, "u1", "online", None, None)
            .await
            .unwrap();
        upsert_presence(&pool, "u1", "idle", Some("AFK"), None)
            .await
            .unwrap();

        let pres = get_presence(&pool, "u1").await.unwrap().unwrap();
        assert_eq!(pres.status, "idle");
        assert_eq!(pres.custom_status, Some("AFK".to_string()));
    }

    #[tokio::test]
    async fn test_set_offline() {
        let pool = setup_db().await;
        create_user(&pool, "u1").await;

        upsert_presence(&pool, "u1", "online", None, None)
            .await
            .unwrap();
        set_offline(&pool, "u1").await.unwrap();

        let pres = get_presence(&pool, "u1").await.unwrap().unwrap();
        assert_eq!(pres.status, "offline");
    }

    #[tokio::test]
    async fn test_get_presences_for_users() {
        let pool = setup_db().await;
        create_user(&pool, "u1").await;
        create_user(&pool, "u2").await;

        upsert_presence(&pool, "u1", "online", None, None)
            .await
            .unwrap();
        upsert_presence(&pool, "u2", "idle", None, None)
            .await
            .unwrap();

        let presences = get_presences_for_users(&pool, &["u1".to_string(), "u2".to_string()])
            .await
            .unwrap();
        assert_eq!(presences.len(), 2);
    }

    #[tokio::test]
    async fn test_get_presences_empty_list() {
        let pool = setup_db().await;
        let presences = get_presences_for_users(&pool, &[]).await.unwrap();
        assert!(presences.is_empty());
    }

    #[tokio::test]
    async fn test_get_nonexistent_presence() {
        let pool = setup_db().await;
        let p = get_presence(&pool, "nosuch").await.unwrap();
        assert!(p.is_none());
    }
}
