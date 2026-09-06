use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AtprotoPublication {
    pub id: String,
    pub status: String,
    pub remote_uri: Option<String>,
    pub remote_cid: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AtprotoPublicationStatus {
    pub id: String,
    pub source_message_id: String,
    pub source_version: i64,
    pub channel_id: String,
    pub status: String,
    pub remote_uri: Option<String>,
    pub remote_cid: Option<String>,
    pub safe_error_code: Option<String>,
    pub updated_at: String,
    pub retryable: bool,
    pub reauthentication_required: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AtprotoChannelPublicationPolicy {
    pub channel_id: String,
    pub eligible: bool,
    pub channel_enabled: bool,
    pub user_granted: bool,
}

pub async fn list_publications(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<AtprotoPublicationStatus>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id,p.source_message_id,p.source_version,m.channel_id,p.status,
                p.remote_uri,p.remote_cid,p.safe_error_code,p.updated_at,
                p.status IN ('failed','cancelled') AS retryable,
                NOT EXISTS(SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=p.user_id
                  AND oa.provider='atproto' AND oa.credential_state='active')
                  AS reauthentication_required
         FROM atproto_publications p JOIN messages m ON m.id=p.source_message_id
         WHERE p.user_id=? ORDER BY p.updated_at DESC,p.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum PublicationRequestError {
    #[error("publication source is unavailable")]
    Unavailable,
    #[error("publication authentication is no longer valid")]
    Authentication,
    #[error("publication dependency is unavailable")]
    DependencyUnavailable,
    #[error("publication storage failed")]
    Database(#[from] sqlx::Error),
}

/// Authorize and durably schedule a public export in one transaction. Remote
/// work re-reads this record and the source visibility before dispatch.
pub async fn request_publication(
    admission: &crate::engine::write_admission::WriteAdmission,
    authorization: &crate::engine::authorization::AuthorizationService,
    auth: &crate::auth::authority::AuthService,
    actor: &crate::auth::authority::Actor,
    message_id: &str,
) -> Result<AtprotoPublication, PublicationRequestError> {
    let (_permit, mut transaction) = admission.begin().await.map_err(|error| match error {
        crate::engine::write_admission::WriteAdmissionError::Database(error) => {
            PublicationRequestError::Database(error)
        }
        crate::engine::write_admission::WriteAdmissionError::Unavailable => {
            PublicationRequestError::DependencyUnavailable
        }
    })?;
    let source = sqlx::query(
        "SELECT m.entity_version,c.id,g.grant_version
         FROM messages m
         JOIN channels c ON c.id=m.channel_id
         JOIN atproto_publication_grants g ON g.user_id=? AND g.channel_id=c.id AND g.enabled=1
         WHERE m.id=? AND m.sender_id=? AND m.deleted_at IS NULL
           AND c.is_private=0 AND c.atproto_publication_enabled=1
           AND c.visibility_repair_required=0 AND c.parent_channel_id IS NULL
           AND c.channel_type NOT IN ('public_thread','private_thread')",
    )
    .bind(actor.user_id().as_str())
    .bind(message_id)
    .bind(actor.user_id().as_str())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(PublicationRequestError::Unavailable)?;
    let source_version: i64 = source.get(0);
    let channel_id: String = source.get(1);
    let grant_version: i64 = source.get(2);
    authorization
        .authorize_actor_in(
            &mut transaction,
            auth,
            actor,
            &channel_id,
            crate::engine::authorization::ChannelAction::View,
        )
        .await
        .map_err(map_publication_authorization_error)?;
    authorization
        .authorize_actor_in(
            &mut transaction,
            auth,
            actor,
            &channel_id,
            crate::engine::authorization::ChannelAction::ReadHistory,
        )
        .await
        .map_err(map_publication_authorization_error)?;

    if let Some(existing) = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT id,status,remote_uri,remote_cid FROM atproto_publications
         WHERE user_id=? AND source_message_id=? AND destination='bluesky'",
    )
    .bind(actor.user_id().as_str())
    .bind(message_id)
    .fetch_optional(&mut *transaction)
    .await?
    {
        transaction.commit().await?;
        return Ok(AtprotoPublication {
            id: existing.0,
            status: existing.1,
            remote_uri: existing.2,
            remote_cid: existing.3,
        });
    }

    let id = Uuid::new_v4().to_string();
    let record_key = id.replace('-', "");
    sqlx::query(
        "INSERT INTO atproto_publications
         (id,user_id,source_message_id,source_version,destination,collection,record_key,status)
         VALUES(?,?,?,?,'bluesky','app.bsky.feed.post',?,'pending')",
    )
    .bind(&id)
    .bind(actor.user_id().as_str())
    .bind(message_id)
    .bind(source_version)
    .bind(&record_key)
    .execute(&mut *transaction)
    .await?;
    let job_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO external_jobs
         (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json)
         VALUES(?,?, 'atproto_publish',?,?,?,?)",
    )
    .bind(job_id)
    .bind(format!("atproto-publication:{id}:{source_version}"))
    .bind(&id)
    .bind(source_version)
    .bind(format!(
        "atproto-user:{}:{grant_version}",
        actor.user_id().as_str()
    ))
    .bind(serde_json::json!({"publication_id": &id, "grant_version": grant_version}).to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(AtprotoPublication {
        id,
        status: "pending".into(),
        remote_uri: None,
        remote_cid: None,
    })
}

/// Requeue a failed/cancelled publication using its stable collection/key.
/// Create and update retries use `putRecord`, so an unknown remote success is
/// reconciled without creating another record.
pub async fn retry_publication(
    admission: &crate::engine::write_admission::WriteAdmission,
    authorization: &crate::engine::authorization::AuthorizationService,
    auth: &crate::auth::authority::AuthService,
    actor: &crate::auth::authority::Actor,
    publication_id: &str,
) -> Result<AtprotoPublication, PublicationRequestError> {
    let (_permit, mut transaction) = admission.begin().await.map_err(|error| match error {
        crate::engine::write_admission::WriteAdmissionError::Database(error) => {
            PublicationRequestError::Database(error)
        }
        crate::engine::write_admission::WriteAdmissionError::Unavailable => {
            PublicationRequestError::DependencyUnavailable
        }
    })?;
    auth.validate_actor_in(&mut transaction, actor)
        .await
        .map_err(|error| match error {
            crate::auth::authority::AuthError::Database(_)
            | crate::auth::authority::AuthError::VerificationBusy
            | crate::auth::authority::AuthError::HashWorker(_) => {
                PublicationRequestError::DependencyUnavailable
            }
            _ => PublicationRequestError::Authentication,
        })?;
    let row = sqlx::query(
        "SELECT p.source_message_id,p.source_version,p.status,p.remote_uri,p.remote_cid,
                m.channel_id,m.deleted_at,g.grant_version,c.atproto_publication_enabled,
                c.is_private,c.visibility_repair_required,c.parent_channel_id,c.channel_type,
                EXISTS(SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=p.user_id
                  AND oa.provider='atproto' AND oa.credential_state='active')
         FROM atproto_publications p JOIN messages m ON m.id=p.source_message_id
         JOIN channels c ON c.id=m.channel_id
         LEFT JOIN atproto_publication_grants g ON g.user_id=p.user_id
              AND g.channel_id=c.id AND g.enabled=1
         WHERE p.id=? AND p.user_id=? AND p.status IN ('failed','cancelled')",
    )
    .bind(publication_id)
    .bind(actor.user_id().as_str())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(PublicationRequestError::Unavailable)?;
    let channel_id: String = row.get(5);
    let deleted = row.get::<Option<String>, _>(6).is_some();
    let remote_uri: Option<String> = row.get(3);
    if !row.get::<bool, _>(13) {
        return Err(PublicationRequestError::Authentication);
    }
    let operation = if deleted {
        "atproto_delete"
    } else {
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(map_publication_authorization_error)?;
        let eligible = row.get::<Option<i64>, _>(7).is_some()
            && row.get::<i64, _>(8) == 1
            && row.get::<i64, _>(9) == 0
            && row.get::<i64, _>(10) == 0
            && row.get::<Option<String>, _>(11).is_none()
            && !matches!(
                row.get::<String, _>(12).as_str(),
                "public_thread" | "private_thread"
            );
        if !eligible {
            return Err(PublicationRequestError::Unavailable);
        }
        if remote_uri.is_some() {
            "atproto_update"
        } else {
            "atproto_publish"
        }
    };
    let source_version: i64 = row.get(1);
    let grant_version = row.get::<Option<i64>, _>(7).unwrap_or(0);
    let status = match operation {
        "atproto_delete" => "delete_pending",
        "atproto_update" => "update_pending",
        _ => "pending",
    };
    let updated = sqlx::query(
        "UPDATE atproto_publications SET status=?,safe_error_code=NULL,updated_at=datetime('now')
         WHERE id=? AND source_version=? AND status IN ('failed','cancelled')",
    )
    .bind(status)
    .bind(publication_id)
    .bind(source_version)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(PublicationRequestError::Unavailable);
    }
    let job_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO external_jobs
         (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json)
         VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&job_id)
    .bind(format!("atproto-publication:{publication_id}:{source_version}:retry:{job_id}"))
    .bind(operation)
    .bind(publication_id)
    .bind(source_version)
    .bind(format!("atproto-user:{}:{grant_version}", actor.user_id().as_str()))
    .bind(serde_json::json!({"publication_id": publication_id, "reconcile": true}).to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(AtprotoPublication {
        id: publication_id.to_owned(),
        status: status.into(),
        remote_uri,
        remote_cid: row.get(4),
    })
}

/// Advance an existing publication alongside its canonical source mutation.
/// `putRecord` makes create/update convergence safe with the retained key, and
/// deletion must be attempted even when an earlier create response was lost.
pub async fn schedule_source_mutation(
    connection: &mut sqlx::SqliteConnection,
    message_id: &str,
    source_version: i64,
    deleted: bool,
) -> Result<(), sqlx::Error> {
    let publication = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT p.id,p.user_id,COALESCE(g.grant_version,0) \
         FROM atproto_publications p \
         JOIN messages m ON m.id=p.source_message_id \
         LEFT JOIN atproto_publication_grants g \
           ON g.user_id=p.user_id AND g.channel_id=m.channel_id AND g.enabled=1 \
         WHERE p.source_message_id=? AND p.status!='deleted' \
           AND (? OR p.status!='cancelled')",
    )
    .bind(message_id)
    .bind(deleted)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((publication_id, user_id, grant_version)) = publication else {
        return Ok(());
    };
    let (operation, status) = if deleted {
        ("atproto_delete", "delete_pending")
    } else {
        ("atproto_update", "update_pending")
    };
    sqlx::query(
        "UPDATE atproto_publications \
         SET source_version=?,status=?,safe_error_code=NULL,updated_at=datetime('now') \
         WHERE id=?",
    )
    .bind(source_version)
    .bind(status)
    .bind(&publication_id)
    .execute(&mut *connection)
    .await?;
    let job_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO external_jobs \
         (id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) \
         VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&job_id)
    // Match the database trigger's key so callers can safely invoke this
    // helper after a canonical message update without scheduling a duplicate.
    .bind(format!(
        "atproto-publication:{publication_id}:{source_version}"
    ))
    .bind(operation)
    .bind(&publication_id)
    .bind(source_version)
    .bind(format!("atproto-user:{user_id}:{grant_version}"))
    .bind(
        serde_json::json!({
            "publication_id": publication_id,
            "source_mutation": true,
        })
        .to_string(),
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn map_publication_authorization_error(
    error: crate::engine::authorization::AuthorizationError,
) -> PublicationRequestError {
    use crate::auth::authority::AuthError;
    use crate::engine::authorization::AuthorizationError;
    match error {
        AuthorizationError::Unavailable => PublicationRequestError::Unavailable,
        AuthorizationError::Database(error) => PublicationRequestError::Database(error),
        AuthorizationError::Authentication(
            AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_),
        ) => PublicationRequestError::DependencyUnavailable,
        AuthorizationError::Authentication(_) => PublicationRequestError::Authentication,
    }
}

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

    #[tokio::test]
    async fn publication_requires_current_public_channel_and_explicit_grant_before_deduplication() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('s1','Test','u1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('s1','u1','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,atproto_publication_enabled) VALUES('c1','s1','#public',1)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('m1','s1','c1','u1','alice','public')")
            .execute(&pool).await.unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("u1").await.unwrap().1;
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        let admission = crate::engine::write_admission::WriteAdmission::new(pool.clone());

        assert!(matches!(
            request_publication(&admission, &authorization, &auth, &actor, "m1").await,
            Err(PublicationRequestError::Unavailable)
        ));
        sqlx::query("INSERT INTO atproto_publication_grants(user_id,channel_id,enabled) VALUES('u1','c1',1)")
            .execute(&pool).await.unwrap();
        let first = request_publication(&admission, &authorization, &auth, &actor, "m1")
            .await
            .unwrap();
        let second = request_publication(&admission, &authorization, &auth, &actor, "m1")
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        let jobs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_jobs WHERE operation_type='atproto_publish'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(jobs, 1);

        sqlx::query("UPDATE atproto_publication_grants SET enabled=0 WHERE user_id='u1' AND channel_id='c1'")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            request_publication(&admission, &authorization, &auth, &actor, "m1").await,
            Err(PublicationRequestError::Unavailable)
        ));
        sqlx::query("UPDATE atproto_publication_grants SET enabled=1; UPDATE channels SET is_private=1 WHERE id='c1'")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            request_publication(&admission, &authorization, &auth, &actor, "m1").await,
            Err(PublicationRequestError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn failed_create_update_and_delete_retry_with_stable_record_identity() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        for statement in [
            "INSERT INTO servers(id,name,owner_id) VALUES('s1','Test','u1')",
            "INSERT INTO server_members(server_id,user_id,role) VALUES('s1','u1','owner')",
            "INSERT INTO channels(id,server_id,name,atproto_publication_enabled) VALUES('c1','s1','#public',1)",
            "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled) VALUES('u1','c1',1)",
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('m1','s1','c1','u1','alice','first')",
            "UPDATE oauth_accounts SET pds_url='https://pds.example',credential_state='active' WHERE user_id='u1' AND provider='atproto'",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("u1").await.unwrap().1;
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        let admission = crate::engine::write_admission::WriteAdmission::new(pool.clone());
        let publication = request_publication(&admission, &authorization, &auth, &actor, "m1")
            .await
            .unwrap();
        let record_key: String =
            sqlx::query_scalar("SELECT record_key FROM atproto_publications WHERE id=?")
                .bind(&publication.id)
                .fetch_one(&pool)
                .await
                .unwrap();

        for (mutation, expected_operation, expected_status) in [
            (None, "atproto_publish", "pending"),
            (
                Some(
                    "UPDATE messages SET content='second',entity_version=entity_version+1 WHERE id='m1'",
                ),
                "atproto_update",
                "update_pending",
            ),
            (
                Some(
                    "UPDATE messages SET deleted_at=datetime('now'),entity_version=entity_version+1 WHERE id='m1'",
                ),
                "atproto_delete",
                "delete_pending",
            ),
        ] {
            if let Some(mutation) = mutation {
                sqlx::query(mutation).execute(&pool).await.unwrap();
            }
            sqlx::query("DELETE FROM external_jobs")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE atproto_publications SET status='failed',safe_error_code='restore_reconciliation_required',remote_uri=CASE WHEN ?='atproto_update' THEN 'at://did:plc:u1/app.bsky.feed.post/stable' ELSE NULL END WHERE id=?")
                .bind(expected_operation).bind(&publication.id).execute(&pool).await.unwrap();
            let retried =
                retry_publication(&admission, &authorization, &auth, &actor, &publication.id)
                    .await
                    .unwrap();
            assert_eq!(retried.status, expected_status);
            let operation: String = sqlx::query_scalar("SELECT operation_type FROM external_jobs")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(operation, expected_operation);
            let current_key: String =
                sqlx::query_scalar("SELECT record_key FROM atproto_publications WHERE id=?")
                    .bind(&publication.id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(current_key, record_key);
        }
    }

    #[tokio::test]
    async fn source_edit_and_delete_advance_one_publication_and_outbox_key() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        for statement in [
            "INSERT INTO servers(id,name,owner_id) VALUES('s1','Test','u1')",
            "INSERT INTO server_members(server_id,user_id,role) VALUES('s1','u1','owner')",
            "INSERT INTO channels(id,server_id,name,atproto_publication_enabled) VALUES('c1','s1','#public',1)",
            "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled) VALUES('u1','c1',1)",
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('m1','s1','c1','u1','alice','first')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("u1").await.unwrap().1;
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        let admission = crate::engine::write_admission::WriteAdmission::new(pool.clone());
        let publication = request_publication(&admission, &authorization, &auth, &actor, "m1")
            .await
            .unwrap();
        let original_key: String =
            sqlx::query_scalar("SELECT record_key FROM atproto_publications WHERE id=?")
                .bind(&publication.id)
                .fetch_one(&pool)
                .await
                .unwrap();

        let mut transaction = pool.begin().await.unwrap();
        sqlx::query("UPDATE messages SET content='edited',entity_version=2 WHERE id='m1'")
            .execute(&mut *transaction)
            .await
            .unwrap();
        schedule_source_mutation(&mut transaction, "m1", 2, false)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let edited: (i64, String, String) = sqlx::query_as(
            "SELECT source_version,status,record_key FROM atproto_publications WHERE id=?",
        )
        .bind(&publication.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(edited, (2, "update_pending".into(), original_key.clone()));

        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(
            "UPDATE messages SET deleted_at=datetime('now'),entity_version=3 WHERE id='m1'",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        schedule_source_mutation(&mut transaction, "m1", 3, true)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let deleted: (i64, String, String) = sqlx::query_as(
            "SELECT source_version,status,record_key FROM atproto_publications WHERE id=?",
        )
        .bind(&publication.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(deleted, (3, "delete_pending".into(), original_key));
        let operations: Vec<String> = sqlx::query_scalar(
            "SELECT operation_type FROM external_jobs WHERE resource_id=? ORDER BY resource_version",
        )
        .bind(&publication.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            operations,
            vec!["atproto_publish", "atproto_update", "atproto_delete"]
        );
    }
}
