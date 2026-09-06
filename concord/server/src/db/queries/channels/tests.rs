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
