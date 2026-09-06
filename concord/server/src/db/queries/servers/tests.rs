use super::*;
use crate::db::pool::{create_pool, run_migrations};
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

#[tokio::test]
async fn test_create_and_get_server() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    create_server(&pool, "s1", "Test Server", "u1", None)
        .await
        .unwrap();

    let server = get_server(&pool, "s1").await.unwrap();
    assert!(server.is_some());
    let s = server.unwrap();
    assert_eq!(s.id, "s1");
    assert_eq!(s.name, "Test Server");
    assert_eq!(s.owner_id, "u1");
    assert!(s.icon_url.is_none());
}

#[tokio::test]
async fn test_create_server_adds_owner_as_member() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_server(&pool, "s1", "Test Server", "u1", None)
        .await
        .unwrap();

    let member = get_server_member(&pool, "s1", "u1").await.unwrap();
    assert!(member.is_some());
    assert_eq!(member.unwrap().role, "owner");
}

#[tokio::test]
async fn test_list_servers_for_user() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_test_user(&pool, "u2", "bob").await;

    create_server(&pool, "s1", "Alpha", "u1", None)
        .await
        .unwrap();
    create_server(&pool, "s2", "Beta", "u1", None)
        .await
        .unwrap();
    create_server(&pool, "s3", "Gamma", "u2", None)
        .await
        .unwrap();

    let user1_servers = list_servers_for_user(&pool, "u1").await.unwrap();
    assert_eq!(user1_servers.len(), 2);
    // Ordered by name
    assert_eq!(user1_servers[0].name, "Alpha");
    assert_eq!(user1_servers[1].name, "Beta");

    let user2_servers = list_servers_for_user(&pool, "u2").await.unwrap();
    assert_eq!(user2_servers.len(), 1);
}

#[tokio::test]
async fn test_add_and_remove_member() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_test_user(&pool, "u2", "bob").await;

    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    // Add member
    add_server_member(&pool, "s1", "u2", "member")
        .await
        .unwrap();
    let count = get_member_count(&pool, "s1").await.unwrap();
    assert_eq!(count, 2); // owner + new member

    let member = get_server_member(&pool, "s1", "u2").await.unwrap();
    assert!(member.is_some());
    assert_eq!(member.unwrap().role, "member");

    // Remove member
    remove_server_member(&pool, "s1", "u2").await.unwrap();
    let count_after = get_member_count(&pool, "s1").await.unwrap();
    assert_eq!(count_after, 1);
}

#[tokio::test]
async fn test_update_member_role() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_test_user(&pool, "u2", "bob").await;

    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();
    add_server_member(&pool, "s1", "u2", "member")
        .await
        .unwrap();

    update_member_role(&pool, "s1", "u2", "admin")
        .await
        .unwrap();

    let member = get_server_member(&pool, "s1", "u2").await.unwrap().unwrap();
    assert_eq!(member.role, "admin");
}

#[tokio::test]
async fn test_update_server() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_server(&pool, "s1", "Old Name", "u1", None)
        .await
        .unwrap();

    update_server(&pool, "s1", "New Name", Some("https://icon.png"))
        .await
        .unwrap();

    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(server.name, "New Name");
    assert_eq!(server.icon_url, Some("https://icon.png".to_string()));
}

#[tokio::test]
async fn test_delete_server() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    delete_server(&pool, "s1").await.unwrap();

    let server = get_server(&pool, "s1").await.unwrap();
    assert!(server.is_none());
}

#[tokio::test]
async fn test_get_nonexistent_server() {
    let pool = setup_db().await;
    let server = get_server(&pool, "nonexistent").await.unwrap();
    assert!(server.is_none());
}

#[tokio::test]
async fn test_list_all_servers() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    create_server(&pool, "s1", "Beta", "u1", None)
        .await
        .unwrap();
    create_server(&pool, "s2", "Alpha", "u1", None)
        .await
        .unwrap();

    let all = list_all_servers(&pool).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].name, "Alpha"); // ordered by name
}

#[tokio::test]
async fn test_system_admin() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    assert!(!is_system_admin(&pool, "u1").await.unwrap());

    set_system_admin(&pool, "u1", true).await.unwrap();
    assert!(is_system_admin(&pool, "u1").await.unwrap());

    set_system_admin(&pool, "u1", false).await.unwrap();
    assert!(!is_system_admin(&pool, "u1").await.unwrap());
}

#[tokio::test]
async fn test_add_member_idempotent() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_test_user(&pool, "u2", "bob").await;
    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    add_server_member(&pool, "s1", "u2", "member")
        .await
        .unwrap();
    // Adding again should not error (INSERT OR IGNORE)
    add_server_member(&pool, "s1", "u2", "member")
        .await
        .unwrap();

    let count = get_member_count(&pool, "s1").await.unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_get_server_members() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_test_user(&pool, "u2", "bob").await;
    create_test_user(&pool, "u3", "charlie").await;
    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    add_server_member(&pool, "s1", "u2", "member")
        .await
        .unwrap();
    add_server_member(&pool, "s1", "u3", "member")
        .await
        .unwrap();

    let members = get_server_members(&pool, "s1").await.unwrap();
    assert_eq!(members.len(), 3);
}

#[tokio::test]
async fn test_set_and_get_server_avatar() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    // Initially no server avatar
    let avatar = get_server_avatar(&pool, "s1", "u1").await.unwrap();
    assert!(avatar.is_none());

    // Set server avatar
    set_server_avatar(&pool, "s1", "u1", Some("https://example.com/avatar.png"))
        .await
        .unwrap();
    let avatar = get_server_avatar(&pool, "s1", "u1").await.unwrap();
    assert_eq!(avatar, Some("https://example.com/avatar.png".to_string()));

    // Clear server avatar
    set_server_avatar(&pool, "s1", "u1", None).await.unwrap();
    let avatar = get_server_avatar(&pool, "s1", "u1").await.unwrap();
    assert!(avatar.is_none());
}

#[tokio::test]
async fn test_update_emoji_settings() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    // Defaults are 1 (enabled)
    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(server.allow_external_emoji, 1);
    assert_eq!(server.shareable_emoji, 1);

    // Disable both
    update_emoji_settings(&pool, "s1", false, false)
        .await
        .unwrap();
    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(server.allow_external_emoji, 0);
    assert_eq!(server.shareable_emoji, 0);

    // Re-enable
    update_emoji_settings(&pool, "s1", true, true)
        .await
        .unwrap();
    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(server.allow_external_emoji, 1);
    assert_eq!(server.shareable_emoji, 1);
}

#[tokio::test]
async fn test_set_vanity_code() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    create_server(&pool, "s1", "Test", "u1", None)
        .await
        .unwrap();

    // Initially no vanity code
    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert!(server.vanity_code.is_none());

    // Set vanity code
    set_vanity_code(&pool, "s1", Some("my-server"))
        .await
        .unwrap();
    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(server.vanity_code, Some("my-server".to_string()));

    // Look up by vanity code
    let found = get_server_by_vanity(&pool, "my-server").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "s1");

    // Non-existent vanity code
    let not_found = get_server_by_vanity(&pool, "other").await.unwrap();
    assert!(not_found.is_none());

    // Clear vanity code
    set_vanity_code(&pool, "s1", None).await.unwrap();
    let server = get_server(&pool, "s1").await.unwrap().unwrap();
    assert!(server.vanity_code.is_none());
}
