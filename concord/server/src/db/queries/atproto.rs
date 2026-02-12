use sqlx::SqlitePool;

/// Parameters for storing a Bluesky profile sync result.
pub struct StoreBskyProfileParams<'a> {
    pub pool: &'a SqlitePool,
    pub user_id: &'a str,
    pub handle: &'a str,
    pub display_name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub banner_url: Option<&'a str>,
    pub followers_count: i64,
    pub follows_count: i64,
}

/// Store a Bluesky profile sync result on the user's oauth_account.
pub async fn store_bsky_profile_sync(p: &StoreBskyProfileParams<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE oauth_accounts SET \
         bsky_handle = ?, bsky_display_name = ?, bsky_description = ?, \
         bsky_banner_url = ?, bsky_followers_count = ?, bsky_follows_count = ?, \
         last_profile_sync = datetime('now') \
         WHERE user_id = ? AND provider = 'atproto'",
    )
    .bind(p.handle)
    .bind(p.display_name)
    .bind(p.description)
    .bind(p.banner_url)
    .bind(p.followers_count)
    .bind(p.follows_count)
    .bind(p.user_id)
    .execute(p.pool)
    .await?;
    Ok(())
}

/// Row type for Bluesky profile sync data.
#[derive(Debug, Clone)]
pub struct BskyProfileSyncRow {
    pub bsky_handle: Option<String>,
    pub bsky_display_name: Option<String>,
    pub bsky_description: Option<String>,
    pub bsky_banner_url: Option<String>,
    pub bsky_followers_count: Option<i64>,
    pub bsky_follows_count: Option<i64>,
    pub last_profile_sync: Option<String>,
    pub did: String,
}

/// Get the Bluesky handle for a user.
pub async fn get_bsky_handle(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_scalar::<_, Option<String>>(
        "SELECT bsky_handle FROM oauth_accounts WHERE user_id = ? AND provider = 'atproto'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.flatten())
}

/// Get full Bluesky profile sync data for a user.
pub async fn get_bsky_profile_sync(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<BskyProfileSyncRow>, sqlx::Error> {
    let row = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<String>,
            String,
        ),
    >(
        "SELECT bsky_handle, bsky_display_name, bsky_description, bsky_banner_url, \
         bsky_followers_count, bsky_follows_count, last_profile_sync, provider_id \
         FROM oauth_accounts WHERE user_id = ? AND provider = 'atproto'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(handle, display_name, description, banner_url, followers, follows, sync, did)| {
            BskyProfileSyncRow {
                bsky_handle: handle,
                bsky_display_name: display_name,
                bsky_description: description,
                bsky_banner_url: banner_url,
                bsky_followers_count: followers,
                bsky_follows_count: follows,
                last_profile_sync: sync,
                did,
            }
        },
    ))
}

/// Insert a record of a shared post to Bluesky (prevents duplicate sharing).
pub async fn insert_shared_post(
    pool: &SqlitePool,
    id: &str,
    message_id: &str,
    user_id: &str,
    at_uri: &str,
    cid: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bsky_shared_posts (id, message_id, user_id, at_uri, cid) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(message_id)
    .bind(user_id)
    .bind(at_uri)
    .bind(cid)
    .execute(pool)
    .await?;
    Ok(())
}

/// Check if a message has already been shared to Bluesky by a user.
pub async fn get_shared_post(
    pool: &SqlitePool,
    message_id: &str,
    user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT at_uri FROM bsky_shared_posts WHERE message_id = ? AND user_id = ?",
    )
    .bind(message_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool(":memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn create_test_user(pool: &SqlitePool, user_id: &str, username: &str) {
        sqlx::query("INSERT INTO users (id, username) VALUES (?, ?)")
            .bind(user_id)
            .bind(username)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO oauth_accounts (id, user_id, provider, provider_id) VALUES (?, ?, 'atproto', ?)",
        )
        .bind(format!("oa_{user_id}"))
        .bind(user_id)
        .bind(format!("did:plc:{user_id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_bsky_profile_sync_roundtrip() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;

        // Initially no handle
        let handle = get_bsky_handle(&pool, "u1").await.unwrap();
        assert!(handle.is_none());

        // Store sync data
        store_bsky_profile_sync(&StoreBskyProfileParams {
            pool: &pool,
            user_id: "u1",
            handle: "alice.bsky.social",
            display_name: Some("Alice"),
            description: Some("Hello world"),
            banner_url: Some("https://banner.example.com/img.jpg"),
            followers_count: 150,
            follows_count: 42,
        })
        .await
        .unwrap();

        // Handle is now set
        let handle = get_bsky_handle(&pool, "u1").await.unwrap();
        assert_eq!(handle.as_deref(), Some("alice.bsky.social"));

        // Full profile sync data
        let sync = get_bsky_profile_sync(&pool, "u1").await.unwrap().unwrap();
        assert_eq!(sync.bsky_handle.as_deref(), Some("alice.bsky.social"));
        assert_eq!(sync.bsky_display_name.as_deref(), Some("Alice"));
        assert_eq!(sync.bsky_description.as_deref(), Some("Hello world"));
        assert_eq!(sync.bsky_followers_count, Some(150));
        assert_eq!(sync.bsky_follows_count, Some(42));
        assert!(sync.last_profile_sync.is_some());
        assert_eq!(sync.did, "did:plc:u1");
    }

    #[tokio::test]
    async fn test_shared_post_insert_and_duplicate() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;

        // Create a message to share
        sqlx::query("INSERT INTO servers (id, name, owner_id) VALUES ('s1', 'Test', 'u1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels (id, server_id, name) VALUES ('c1', 's1', 'general')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, server_id, channel_id, sender_id, sender_nick, content) \
             VALUES ('m1', 's1', 'c1', 'u1', 'alice', 'Hello!')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // No shared post yet
        let uri = get_shared_post(&pool, "m1", "u1").await.unwrap();
        assert!(uri.is_none());

        // Insert shared post
        insert_shared_post(
            &pool,
            "sp1",
            "m1",
            "u1",
            "at://did:plc:u1/app.bsky.feed.post/abc",
            "bafyreiabc",
        )
        .await
        .unwrap();

        // Now it exists
        let uri = get_shared_post(&pool, "m1", "u1").await.unwrap();
        assert_eq!(
            uri.as_deref(),
            Some("at://did:plc:u1/app.bsky.feed.post/abc")
        );

        // Duplicate insert fails (UNIQUE constraint)
        let result = insert_shared_post(&pool, "sp2", "m1", "u1", "at://other", "bafyother").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bsky_profile_sync_no_user() {
        let pool = setup_db().await;
        let sync = get_bsky_profile_sync(&pool, "nonexistent").await.unwrap();
        assert!(sync.is_none());
    }
}
