use super::*;
use crate::db::pool::{create_pool, run_migrations};
use crate::db::queries::channels;
use crate::db::queries::servers;
use crate::db::queries::users::{self, CreateOAuthUser};

async fn setup_db() -> SqlitePool {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

async fn setup_server_and_channel(pool: &SqlitePool) {
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
    channels::ensure_channel(pool, "c1", "s1", "#general")
        .await
        .unwrap();
}

fn msg_params<'a>(id: &'a str, content: &'a str) -> InsertMessageParams<'a> {
    InsertMessageParams {
        id,
        server_id: "s1",
        channel_id: "c1",
        sender_id: "u1",
        sender_nick: "alice",
        content,
        reply_to_id: None,
    }
}

#[tokio::test]
async fn test_insert_and_get_message() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;

    insert_message(&pool, &msg_params("m1", "Hello world"))
        .await
        .unwrap();

    let msg = get_message_by_id(&pool, "m1").await.unwrap();
    assert!(msg.is_some());
    let m = msg.unwrap();
    assert_eq!(m.id, "m1");
    assert_eq!(m.content, "Hello world");
    assert_eq!(m.sender_nick, "alice");
    assert!(m.edited_at.is_none());
    assert!(m.deleted_at.is_none());
    assert!(m.reply_to_id.is_none());
}

#[tokio::test]
async fn test_get_nonexistent_message() {
    let pool = setup_db().await;
    let msg = get_message_by_id(&pool, "no-such").await.unwrap();
    assert!(msg.is_none());
}

#[tokio::test]
async fn test_edit_message() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;
    insert_message(&pool, &msg_params("m1", "Original"))
        .await
        .unwrap();

    let edited = update_message_content(&pool, "m1", "Edited").await.unwrap();
    assert!(edited);

    let msg = get_message_by_id(&pool, "m1").await.unwrap().unwrap();
    assert_eq!(msg.content, "Edited");
    assert!(msg.edited_at.is_some());
}

#[tokio::test]
async fn test_edit_deleted_message_fails() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;
    insert_message(&pool, &msg_params("m1", "Hello"))
        .await
        .unwrap();

    soft_delete_message(&pool, "m1").await.unwrap();

    // Editing a deleted message should return false
    let edited = update_message_content(&pool, "m1", "New content")
        .await
        .unwrap();
    assert!(!edited);
}

#[tokio::test]
async fn test_soft_delete_message() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;
    insert_message(&pool, &msg_params("m1", "Hello"))
        .await
        .unwrap();

    let deleted = soft_delete_message(&pool, "m1").await.unwrap();
    assert!(deleted);

    let msg = get_message_by_id(&pool, "m1").await.unwrap().unwrap();
    assert!(msg.deleted_at.is_some());

    // Double-delete should return false
    let deleted_again = soft_delete_message(&pool, "m1").await.unwrap();
    assert!(!deleted_again);
}

#[tokio::test]
async fn test_message_with_reply() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;

    insert_message(&pool, &msg_params("m1", "Hello"))
        .await
        .unwrap();

    let reply_params = InsertMessageParams {
        id: "m2",
        server_id: "s1",
        channel_id: "c1",
        sender_id: "u1",
        sender_nick: "alice",
        content: "Reply!",
        reply_to_id: Some("m1"),
    };
    insert_message(&pool, &reply_params).await.unwrap();

    let reply = get_message_by_id(&pool, "m2").await.unwrap().unwrap();
    assert_eq!(reply.reply_to_id, Some("m1".to_string()));
}

#[tokio::test]
async fn test_fetch_channel_history() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;

    for i in 0..5 {
        insert_message(&pool, &msg_params(&format!("m{i}"), &format!("Msg {i}")))
            .await
            .unwrap();
    }

    // Fetch all (no cursor)
    let history = fetch_channel_history(&pool, "c1", None, 10).await.unwrap();
    assert_eq!(history.len(), 5);
    // Should be newest first
}

#[tokio::test]
async fn test_fetch_channel_history_with_limit() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;

    for i in 0..5 {
        insert_message(&pool, &msg_params(&format!("m{i}"), &format!("Msg {i}")))
            .await
            .unwrap();
    }

    let history = fetch_channel_history(&pool, "c1", None, 2).await.unwrap();
    assert_eq!(history.len(), 2);
}

#[tokio::test]
async fn test_fetch_history_excludes_deleted() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;

    insert_message(&pool, &msg_params("m1", "Keep"))
        .await
        .unwrap();
    insert_message(&pool, &msg_params("m2", "Delete me"))
        .await
        .unwrap();

    soft_delete_message(&pool, "m2").await.unwrap();

    let history = fetch_channel_history(&pool, "c1", None, 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "Keep");
}

#[tokio::test]
async fn test_reactions() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;
    insert_message(&pool, &msg_params("m1", "Hello"))
        .await
        .unwrap();

    // Add reaction
    let added = add_reaction(&pool, "m1", "u1", "thumbsup").await.unwrap();
    assert!(added);

    // Duplicate reaction should not add
    let dup = add_reaction(&pool, "m1", "u1", "thumbsup").await.unwrap();
    assert!(!dup);

    // Get reactions
    let reactions = get_reactions_for_messages(&pool, &["m1".to_string()])
        .await
        .unwrap();
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "thumbsup");

    // Remove reaction
    let removed = remove_reaction(&pool, "m1", "u1", "thumbsup")
        .await
        .unwrap();
    assert!(removed);

    let reactions = get_reactions_for_messages(&pool, &["m1".to_string()])
        .await
        .unwrap();
    assert!(reactions.is_empty());
}

#[tokio::test]
async fn test_reactions_empty_ids() {
    let pool = setup_db().await;
    let reactions = get_reactions_for_messages(&pool, &[]).await.unwrap();
    assert!(reactions.is_empty());
}

#[tokio::test]
async fn test_insert_dm() {
    let pool = setup_db().await;
    users::create_with_oauth(
        &pool,
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
        &pool,
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

    insert_dm(&pool, "dm1", "u1", "alice", "u2", "Hey Bob!")
        .await
        .unwrap();

    let msg = get_message_by_id(&pool, "dm1").await.unwrap().unwrap();
    assert_eq!(msg.target_user_id, Some("u2".to_string()));
    assert_eq!(msg.content, "Hey Bob!");
    assert!(msg.server_id.is_none());
}

#[tokio::test]
async fn test_mark_channel_read() {
    let pool = setup_db().await;
    setup_server_and_channel(&pool).await;
    insert_message(&pool, &msg_params("m1", "Hello"))
        .await
        .unwrap();

    // Mark as read -- should not error
    mark_channel_read(&pool, "u1", "c1", "m1").await.unwrap();

    // Upsert again with new message
    insert_message(&pool, &msg_params("m2", "World"))
        .await
        .unwrap();
    mark_channel_read(&pool, "u1", "c1", "m2").await.unwrap();
}
