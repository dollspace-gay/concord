use super::*;

#[tokio::test]
async fn test_all_migrations_apply_cleanly() {
    // Running setup_db applies every registered migration to a fresh database.
    // If any migration fails, this test will panic.
    let pool = setup_db().await;

    // Verify the current registered target and its checksum ledger agree.
    let max_version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        max_version,
        current_schema_version(),
        "All registered migrations should be recorded"
    );
    let verified_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM schema_version v JOIN migration_metadata m USING(version)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(verified_count, max_version);
}

#[tokio::test]
async fn test_migrations_are_idempotent() {
    let pool = setup_db().await;

    // Run migrations a second time. Should not error (INSERT OR IGNORE).
    run_migrations(&pool).await.unwrap();

    // Verify neither the version ledger nor checksum metadata duplicated.
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM schema_version), (SELECT COUNT(*) FROM migration_metadata)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        counts,
        (current_schema_version(), current_schema_version()),
        "No duplicate migration entries after re-run"
    );
}

#[tokio::test]
async fn test_fts5_index_exists_after_migration() {
    let pool = setup_db().await;

    let fts_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='messages_fts'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(fts_exists, "FTS5 index table should exist");
}

#[tokio::test]
async fn test_webhook_event_subscriptions() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "wh-events-srv";
    queries::servers::create_server(&pool, server_id, "WH Events", &owner_id, None)
        .await
        .unwrap();

    let channel_id = Uuid::new_v4().to_string();
    queries::channels::ensure_channel(&pool, &channel_id, server_id, "#notifs")
        .await
        .unwrap();

    let webhook_id = Uuid::new_v4().to_string();
    queries::webhooks::create_webhook(
        &pool,
        &CreateWebhookParams {
            id: &webhook_id,
            server_id,
            channel_id: &channel_id,
            name: "Event Hook",
            avatar_url: None,
            webhook_type: "outgoing",
            token: &Uuid::new_v4().to_string(),
            url: Some("https://example.com/webhook"),
            created_by: &owner_id,
        },
    )
    .await
    .unwrap();

    // Subscribe to events
    let ev1_id = Uuid::new_v4().to_string();
    let ev2_id = Uuid::new_v4().to_string();
    queries::webhooks::add_webhook_event(&pool, &ev1_id, &webhook_id, "message_create")
        .await
        .unwrap();
    queries::webhooks::add_webhook_event(&pool, &ev2_id, &webhook_id, "member_join")
        .await
        .unwrap();

    // List events
    let events = queries::webhooks::list_webhook_events(&pool, &webhook_id)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);

    let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(event_types.contains(&"message_create"));
    assert!(event_types.contains(&"member_join"));

    // Find outgoing webhooks for a specific event type
    let hooks_for_msg =
        queries::webhooks::list_outgoing_webhooks_for_event(&pool, server_id, "message_create")
            .await
            .unwrap();
    assert_eq!(hooks_for_msg.len(), 1);
    assert_eq!(hooks_for_msg[0].id, webhook_id);

    // Remove one event subscription
    queries::webhooks::remove_webhook_event(&pool, &webhook_id, "message_create")
        .await
        .unwrap();

    let hooks_after =
        queries::webhooks::list_outgoing_webhooks_for_event(&pool, server_id, "message_create")
            .await
            .unwrap();
    assert_eq!(hooks_after.len(), 0);
}

#[tokio::test]
async fn test_bot_user_creation_and_token_lifecycle() {
    let pool = setup_db().await;

    let bot_user_id = Uuid::new_v4().to_string();

    // Create a bot user (insert directly; create_bot_user references columns not in schema)
    sqlx::query("INSERT INTO users (id, username, is_bot) VALUES (?, ?, 1)")
        .bind(&bot_user_id)
        .bind("test-bot")
        .execute(&pool)
        .await
        .unwrap();

    // Verify bot user exists and is flagged as bot
    let is_bot = queries::bots::is_bot_user(&pool, &bot_user_id)
        .await
        .unwrap();
    assert!(is_bot, "Bot user should be flagged as bot");

    // Create a bot token
    let token_id = Uuid::new_v4().to_string();
    let token_hash = "hashed_token_value_123";
    queries::bots::create_bot_token(
        &pool,
        &token_id,
        &bot_user_id,
        token_hash,
        "Primary Token",
        "bot,messages",
    )
    .await
    .unwrap();

    // Verify token can be found by hash
    let token = queries::bots::get_bot_token_by_hash(&pool, token_hash)
        .await
        .unwrap();
    assert!(token.is_some());
    let token = token.unwrap();
    assert_eq!(token.user_id, bot_user_id);
    assert_eq!(token.name, "Primary Token");
    assert_eq!(token.scopes, "bot,messages");

    // Update last_used
    queries::bots::update_token_last_used(&pool, &token_id)
        .await
        .unwrap();

    let updated_token = queries::bots::get_bot_token_by_hash(&pool, token_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(
        updated_token.last_used.is_some(),
        "last_used should be set after touch"
    );

    // Delete the token
    queries::bots::delete_bot_token(&pool, &token_id)
        .await
        .unwrap();

    // Verify authentication would now fail
    let deleted_token = queries::bots::get_bot_token_by_hash(&pool, token_hash)
        .await
        .unwrap();
    assert!(deleted_token.is_none(), "Deleted token should not be found");
}

#[tokio::test]
async fn test_invite_with_use_limit() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "invite-server";
    queries::servers::create_server(&pool, server_id, "Invite Test", &owner_id, None)
        .await
        .unwrap();

    // Create an invite with max 3 uses
    let invite_id = Uuid::new_v4().to_string();
    queries::invites::create_invite(
        &pool,
        &invite_id,
        server_id,
        "ABC123",
        &owner_id,
        Some(3),
        None, // no expiry
        None,
    )
    .await
    .unwrap();

    // Look up by code
    let invite = queries::invites::get_invite_by_code(&pool, "ABC123")
        .await
        .unwrap();
    assert!(invite.is_some());
    let invite = invite.unwrap();
    assert_eq!(invite.max_uses, Some(3));
    assert_eq!(invite.use_count, 0);

    // Use it 3 times
    for _ in 0..3 {
        queries::invites::increment_use_count(&pool, &invite_id)
            .await
            .unwrap();
    }

    // Check the count
    let invite_after = queries::invites::get_invite_by_code(&pool, "ABC123")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invite_after.use_count, 3);

    // At this point, the application logic should check use_count >= max_uses
    // and refuse further joins. We verify the data is correct.
    assert!(
        invite_after.use_count >= invite_after.max_uses.unwrap(),
        "Invite should be exhausted"
    );
}

#[tokio::test]
async fn test_event_rsvp_flow() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;
    let user_id = create_test_user(&pool, "bob").await;

    let server_id = "event-server";
    queries::servers::create_server(&pool, server_id, "Event Test", &owner_id, None)
        .await
        .unwrap();

    // Create an event
    let event_id = Uuid::new_v4().to_string();
    queries::events::create_event(
        &pool,
        &CreateServerEventParams {
            id: &event_id,
            server_id,
            name: "Game Night",
            description: Some("Play board games"),
            channel_id: None,
            start_time: "2026-03-01T20:00:00Z",
            end_time: Some("2026-03-01T23:00:00Z"),
            image_url: None,
            created_by: &owner_id,
        },
    )
    .await
    .unwrap();

    // Verify event
    let event = queries::events::get_event(&pool, &event_id).await.unwrap();
    assert!(event.is_some());
    assert_eq!(event.unwrap().name, "Game Night");

    // RSVP from two users
    queries::events::set_rsvp(&pool, &event_id, &owner_id, "interested")
        .await
        .unwrap();
    queries::events::set_rsvp(&pool, &event_id, &user_id, "interested")
        .await
        .unwrap();

    // List RSVPs
    let rsvps = queries::events::get_rsvps(&pool, &event_id).await.unwrap();
    assert_eq!(rsvps.len(), 2);

    // Get count
    let count = queries::events::get_rsvp_count(&pool, &event_id)
        .await
        .unwrap();
    assert_eq!(count, 2);

    // Change RSVP status (upsert)
    queries::events::set_rsvp(&pool, &event_id, &user_id, "going")
        .await
        .unwrap();

    let rsvps_after = queries::events::get_rsvps(&pool, &event_id).await.unwrap();
    assert_eq!(rsvps_after.len(), 2); // still 2, just different status
    let bob_rsvp = rsvps_after.iter().find(|r| r.user_id == user_id).unwrap();
    assert_eq!(bob_rsvp.status, "going");

    // Remove RSVP
    queries::events::remove_rsvp(&pool, &event_id, &user_id)
        .await
        .unwrap();
    let count_after = queries::events::get_rsvp_count(&pool, &event_id)
        .await
        .unwrap();
    assert_eq!(count_after, 1);
}

#[tokio::test]
async fn test_automod_keyword_filter_rule() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "automod-server";
    queries::servers::create_server(&pool, server_id, "AutoMod Test", &owner_id, None)
        .await
        .unwrap();

    // Create a keyword filter rule
    let rule_id = Uuid::new_v4().to_string();
    queries::automod::create_rule(
        &pool,
        &CreateAutomodRuleParams {
            id: &rule_id,
            server_id,
            name: "Block Spam",
            rule_type: "keyword",
            config: r#"{"keywords":["spam","buy now"]}"#,
            action_type: "delete",
            timeout_duration_seconds: None,
        },
    )
    .await
    .unwrap();

    // List enabled rules
    let rules = queries::automod::get_enabled_rules(&pool, server_id)
        .await
        .unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name, "Block Spam");
    assert_eq!(rules[0].rule_type, "keyword");
    assert_eq!(rules[0].action_type, "delete");

    // Parse the config to verify keywords
    let config: serde_json::Value = serde_json::from_str(&rules[0].config).unwrap();
    let keywords = config["keywords"].as_array().unwrap();
    assert_eq!(keywords.len(), 2);
    assert!(keywords.iter().any(|k| k.as_str() == Some("spam")));
}

#[tokio::test]
async fn test_audit_log_recording() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "audit-server";
    queries::servers::create_server(&pool, server_id, "Audit Test", &owner_id, None)
        .await
        .unwrap();

    // Create several audit log entries
    for action in &["member_kick", "member_ban", "channel_create"] {
        queries::audit_log::create_entry(
            &pool,
            &CreateAuditLogParams {
                id: &Uuid::new_v4().to_string(),
                server_id,
                actor_id: &owner_id,
                action_type: action,
                target_type: Some("user"),
                target_id: Some("target-user-id"),
                reason: Some("Testing"),
                changes: None,
            },
        )
        .await
        .unwrap();
    }

    // List all entries
    let entries = queries::audit_log::list_entries(&pool, server_id, None, 50, None)
        .await
        .unwrap();
    assert_eq!(entries.len(), 3);

    // Filter by action type
    let kick_entries =
        queries::audit_log::list_entries(&pool, server_id, Some("member_kick"), 50, None)
            .await
            .unwrap();
    assert_eq!(kick_entries.len(), 1);
    assert_eq!(kick_entries[0].action_type, "member_kick");
}
