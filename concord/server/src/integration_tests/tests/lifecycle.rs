use super::*;

#[tokio::test]
async fn test_core_tables_created() {
    let pool = setup_db().await;

    let tables = &[
        "users",
        "oauth_accounts",
        "servers",
        "server_members",
        "channels",
        "channel_members",
        "messages",
        "reactions",
        "roles",
        "user_roles",
        "channel_permission_overrides",
        "channel_categories",
        "bans",
        "audit_log",
        "automod_rules",
        "invites",
        "server_events",
        "event_rsvps",
        "channel_follows",
        "server_templates",
        "webhooks",
        "webhook_events",
        "bot_tokens",
        "slash_commands",
    ];

    for table in tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name = ?",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "Table '{table}' should exist after migrations");
    }
}

#[tokio::test]
async fn test_webhook_create_execute_delete_lifecycle() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "wh-server";
    queries::servers::create_server(&pool, server_id, "Webhook Server", &owner_id, None)
        .await
        .unwrap();

    let channel_id = Uuid::new_v4().to_string();
    queries::channels::ensure_channel(&pool, &channel_id, server_id, "#hooks")
        .await
        .unwrap();

    // Create a webhook
    let webhook_id = Uuid::new_v4().to_string();
    let webhook_token = Uuid::new_v4().to_string();
    queries::webhooks::create_webhook(
        &pool,
        &CreateWebhookParams {
            id: &webhook_id,
            server_id,
            channel_id: &channel_id,
            name: "My Webhook",
            avatar_url: None,
            webhook_type: "incoming",
            token: &webhook_token,
            url: None,
            created_by: &owner_id,
        },
    )
    .await
    .unwrap();

    // Verify the webhook exists
    let wh = queries::webhooks::get_webhook(&pool, &webhook_id)
        .await
        .unwrap();
    assert!(wh.is_some());
    let wh = wh.unwrap();
    assert_eq!(wh.name, "My Webhook");
    assert_eq!(wh.webhook_type, "incoming");

    // Look up by token
    let wh_by_token = queries::webhooks::get_webhook_by_token(&pool, &webhook_token)
        .await
        .unwrap();
    assert!(wh_by_token.is_some());
    assert_eq!(wh_by_token.unwrap().id, webhook_id);

    // Delete the webhook
    queries::webhooks::delete_webhook(&pool, &webhook_id)
        .await
        .unwrap();

    // Verify it's gone
    let wh_after = queries::webhooks::get_webhook(&pool, &webhook_id)
        .await
        .unwrap();
    assert!(wh_after.is_none());

    // Token lookup should also fail
    let wh_token_after = queries::webhooks::get_webhook_by_token(&pool, &webhook_token)
        .await
        .unwrap();
    assert!(wh_token_after.is_none());
}

#[tokio::test]
async fn test_delete_server_cascades_to_channels_and_roles() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "cascade-server";
    queries::servers::create_server(&pool, server_id, "Cascade Test", &owner_id, None)
        .await
        .unwrap();

    // Create channels
    let ch1 = Uuid::new_v4().to_string();
    let ch2 = Uuid::new_v4().to_string();
    queries::channels::ensure_channel(&pool, &ch1, server_id, "#general")
        .await
        .unwrap();
    queries::channels::ensure_channel(&pool, &ch2, server_id, "#random")
        .await
        .unwrap();

    // Create a role
    let role_id = Uuid::new_v4().to_string();
    queries::roles::create_role(
        &pool,
        &queries::roles::CreateRoleParams {
            id: &role_id,
            server_id,
            name: "Test Role",
            color: None,
            icon_url: None,
            position: 0,
            permissions: DEFAULT_EVERYONE.bits() as i64,
            is_default: false,
        },
    )
    .await
    .unwrap();

    // Create a message
    let msg_id = Uuid::new_v4().to_string();
    queries::messages::insert_message(
        &pool,
        &queries::messages::InsertMessageParams {
            id: &msg_id,
            server_id,
            channel_id: &ch1,
            sender_id: &owner_id,
            sender_nick: "alice",
            content: "Test",
            reply_to_id: None,
        },
    )
    .await
    .unwrap();

    // Ensure data exists before delete
    let channels_before = queries::channels::list_channels(&pool, server_id)
        .await
        .unwrap();
    assert_eq!(channels_before.len(), 2);
    let roles_before = queries::roles::list_roles(&pool, server_id).await.unwrap();
    assert_eq!(roles_before.len(), 1);

    // Enable foreign keys for cascade
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);

    // Delete the server
    queries::servers::delete_server(&pool, server_id)
        .await
        .unwrap();

    // Verify cascaded deletes
    let server_after = queries::servers::get_server(&pool, server_id)
        .await
        .unwrap();
    assert!(server_after.is_none(), "Server should be deleted");

    let channels_after = queries::channels::list_channels(&pool, server_id)
        .await
        .unwrap();
    assert!(
        channels_after.is_empty(),
        "Channels should cascade delete with server"
    );

    let roles_after = queries::roles::list_roles(&pool, server_id).await.unwrap();
    assert!(
        roles_after.is_empty(),
        "Roles should cascade delete with server"
    );

    let members_after = queries::servers::get_server_members(&pool, server_id)
        .await
        .unwrap();
    assert!(
        members_after.is_empty(),
        "Members should cascade delete with server"
    );
}

#[tokio::test]
async fn test_engine_delete_server_cleans_memory() {
    let (engine, pool) = setup_engine().await;

    let user_id = create_test_user(&pool, "alice").await;
    let server_id = engine
        .create_server_for_actor(&actor_for(&pool, &user_id).await, "To Delete".into(), None)
        .await
        .unwrap();

    // Verify in memory
    assert!(engine.get_server_name(&server_id).is_some());
    assert_eq!(engine.list_channels(&server_id).len(), 1);

    // Delete
    engine.delete_server(&server_id).await.unwrap();

    // Verify cleaned up
    assert!(engine.get_server_name(&server_id).is_none());
    assert_eq!(engine.list_channels(&server_id).len(), 0);
}
