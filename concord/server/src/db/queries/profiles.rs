use sqlx::SqlitePool;

use crate::db::models::UserProfileRow;

/// Upsert a user's profile.
pub async fn upsert_profile(
    pool: &SqlitePool,
    user_id: &str,
    bio: Option<&str>,
    pronouns: Option<&str>,
    banner_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_profiles (user_id, bio, pronouns, banner_url, updated_at) \
         VALUES (?, ?, ?, ?, datetime('now')) \
         ON CONFLICT(user_id) DO UPDATE SET \
         bio = excluded.bio, pronouns = excluded.pronouns, \
         banner_url = excluded.banner_url, updated_at = datetime('now')",
    )
    .bind(user_id)
    .bind(bio)
    .bind(pronouns)
    .bind(banner_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a user's profile.
pub async fn get_profile(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<UserProfileRow>, sqlx::Error> {
    sqlx::query_as::<_, UserProfileRow>(
        "SELECT user_id, bio, pronouns, banner_url, created_at, updated_at \
         FROM user_profiles WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
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

    async fn create_user(pool: &SqlitePool) {
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
    }

    #[tokio::test]
    async fn test_upsert_and_get_profile() {
        let pool = setup_db().await;
        create_user(&pool).await;

        upsert_profile(&pool, "u1", Some("Hello world"), Some("she/her"), None)
            .await
            .unwrap();

        let prof = get_profile(&pool, "u1").await.unwrap();
        assert!(prof.is_some());
        let p = prof.unwrap();
        assert_eq!(p.bio, Some("Hello world".to_string()));
        assert_eq!(p.pronouns, Some("she/her".to_string()));
        assert!(p.banner_url.is_none());
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let pool = setup_db().await;
        create_user(&pool).await;

        upsert_profile(&pool, "u1", Some("Old bio"), None, None)
            .await
            .unwrap();
        upsert_profile(
            &pool,
            "u1",
            Some("New bio"),
            Some("they/them"),
            Some("https://banner.png"),
        )
        .await
        .unwrap();

        let p = get_profile(&pool, "u1").await.unwrap().unwrap();
        assert_eq!(p.bio, Some("New bio".to_string()));
        assert_eq!(p.pronouns, Some("they/them".to_string()));
        assert_eq!(p.banner_url, Some("https://banner.png".to_string()));
    }

    #[tokio::test]
    async fn test_profile_all_null_fields() {
        let pool = setup_db().await;
        create_user(&pool).await;

        upsert_profile(&pool, "u1", None, None, None).await.unwrap();

        let p = get_profile(&pool, "u1").await.unwrap().unwrap();
        assert!(p.bio.is_none());
        assert!(p.pronouns.is_none());
        assert!(p.banner_url.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_profile() {
        let pool = setup_db().await;
        let p = get_profile(&pool, "nosuch").await.unwrap();
        assert!(p.is_none());
    }
}
