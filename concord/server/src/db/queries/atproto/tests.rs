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
    sqlx::query(
        "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled) VALUES('u1','c1',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
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

    sqlx::query(
        "UPDATE atproto_publication_grants SET enabled=0 WHERE user_id='u1' AND channel_id='c1'",
    )
    .execute(&pool)
    .await
    .unwrap();
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
        let retried = retry_publication(&admission, &authorization, &auth, &actor, &publication.id)
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
    sqlx::query("UPDATE messages SET deleted_at=datetime('now'),entity_version=3 WHERE id='m1'")
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
