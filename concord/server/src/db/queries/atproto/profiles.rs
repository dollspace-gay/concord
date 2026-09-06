use super::SqlitePool;

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
