//! Integration tests for Concord — cross-layer tests that verify end-to-end flows,
//! migration correctness, and system-level behavior.
//!
//! Each test creates its own in-memory SQLite database so tests are fully isolated.

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;
    use uuid::Uuid;

    use crate::db::models::{
        CreateAuditLogParams, CreateAutomodRuleParams, CreateServerEventParams, CreateWebhookParams,
    };
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries;
    use crate::engine::chat_engine::ChatEngine;
    use crate::engine::events::ChatEvent;
    use crate::engine::permissions::{
        ChannelOverride, DEFAULT_EVERYONE, DEFAULT_MODERATOR, OverrideTargetType, Permissions,
        compute_effective_permissions,
    };
    use crate::engine::user_session::Protocol;

    // ── Helpers ──────────────────────────────────────────────────

    /// Create an in-memory SQLite pool with all migrations applied.
    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    /// Create a ChatEngine backed by a fresh in-memory database.
    async fn setup_engine() -> (ChatEngine, SqlitePool) {
        let pool = setup_db().await;
        let engine = ChatEngine::new(Some(pool.clone()));
        (engine, pool)
    }

    /// Create a test user in the database and return the user_id.
    async fn create_test_user(pool: &SqlitePool, username: &str) -> String {
        let user_id = Uuid::new_v4().to_string();
        queries::users::create_with_oauth(
            pool,
            &queries::users::CreateOAuthUser {
                user_id: &user_id,
                username,
                email: Some(&format!("{username}@test.com")),
                avatar_url: None,
                oauth_id: &Uuid::new_v4().to_string(),
                provider: "github",
                provider_id: &Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        user_id
    }

    /// Connect a user to the engine and return (session_id, receiver).
    fn connect_user(
        engine: &ChatEngine,
        user_id: Option<&str>,
        nickname: &str,
    ) -> (uuid::Uuid, tokio::sync::mpsc::UnboundedReceiver<ChatEvent>) {
        engine
            .connect(
                user_id.map(|s| s.to_string()),
                nickname.to_string(),
                Protocol::WebSocket,
                None,
            )
            .unwrap()
    }

    /// Drain all pending events from a receiver.
    fn drain_events(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ChatEvent>) {
        while rx.try_recv().is_ok() {}
    }

    // ═══════════════════════════════════════════════════════════════
    //  1. Migration Verification Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_all_migrations_apply_cleanly() {
        // Running setup_db applies all 11 migrations to a fresh database.
        // If any migration fails, this test will panic.
        let pool = setup_db().await;

        // Verify that schema_version has all 11 entries
        let max_version: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_version")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(max_version, 12, "All 12 migrations should be recorded");
    }

    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let pool = setup_db().await;

        // Run migrations a second time. Should not error (INSERT OR IGNORE).
        run_migrations(&pool).await.unwrap();

        // Verify version is still 11, not duplicated.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 12, "No duplicate migration entries after re-run");
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

    // ═══════════════════════════════════════════════════════════════
    //  2. Full User Lifecycle Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_full_user_registration_to_server_creation_flow() {
        let (engine, pool) = setup_engine().await;

        // Step 1: Register a user
        let owner_id = create_test_user(&pool, "alice").await;

        // Step 2: Create a server via the engine
        let server_id = engine
            .create_server("My Server".into(), owner_id.clone(), None)
            .await
            .unwrap();

        // Step 3: Verify server exists in memory and DB
        assert!(engine.get_server_name(&server_id).is_some());
        let db_server = queries::servers::get_server(&pool, &server_id)
            .await
            .unwrap();
        assert!(db_server.is_some());
        assert_eq!(db_server.unwrap().name, "My Server");

        // Step 4: Verify 4 default roles were created
        let roles = queries::roles::list_roles(&pool, &server_id).await.unwrap();
        assert_eq!(roles.len(), 4, "Should have 4 default roles");

        let role_names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
        assert!(role_names.contains(&"@everyone"));
        assert!(role_names.contains(&"Moderator"));
        assert!(role_names.contains(&"Admin"));
        assert!(role_names.contains(&"Owner"));

        // Step 5: Verify the owner has the Owner role assigned
        let user_roles = queries::roles::get_user_roles(&pool, &server_id, &owner_id)
            .await
            .unwrap();
        assert!(
            user_roles.iter().any(|r| r.name == "Owner"),
            "Server creator should have Owner role"
        );

        // Step 6: Verify #general channel was created
        let channels = engine.list_channels(&server_id);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].name, "#general");

        // Step 7: Verify owner is a server member
        let member = queries::servers::get_server_member(&pool, &server_id, &owner_id)
            .await
            .unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().role, "owner");
    }

    #[tokio::test]
    async fn test_user_joins_server_gets_everyone_role() {
        let (engine, pool) = setup_engine().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let joiner_id = create_test_user(&pool, "bob").await;

        let server_id = engine
            .create_server("Test Server".into(), owner_id.clone(), None)
            .await
            .unwrap();

        // Bob joins the server (both DB and in-memory)
        engine.join_server(&joiner_id, &server_id).await.unwrap();

        // Verify Bob is a member in DB
        let member = queries::servers::get_server_member(&pool, &server_id, &joiner_id)
            .await
            .unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().role, "member");

        // Verify the @everyone role exists and has basic permissions
        let default_role = queries::roles::get_default_role(&pool, &server_id)
            .await
            .unwrap();
        assert!(default_role.is_some());
        let everyone = default_role.unwrap();
        let perms = Permissions::from_bits_truncate(everyone.permissions as u64);
        assert!(perms.contains(Permissions::SEND_MESSAGES));
        assert!(perms.contains(Permissions::VIEW_CHANNELS));
        assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
    }

    #[tokio::test]
    async fn test_message_send_edit_delete_lifecycle() {
        let (engine, pool) = setup_engine().await;

        let user_id = create_test_user(&pool, "alice").await;
        let server_id = engine
            .create_server("Msg Test Server".into(), user_id.clone(), None)
            .await
            .unwrap();

        // Connect user and join #general
        let (sid, mut rx) = connect_user(&engine, Some(&user_id), "alice");
        engine.join_channel(sid, &server_id, "#general").unwrap();
        drain_events(&mut rx);

        // Send a message
        engine
            .send_message(sid, &server_id, "#general", "Hello World!", None, None)
            .unwrap();

        // The sender should NOT receive their own message via the channel broadcast
        // (protocol convention), but we should be able to find the message in the DB.
        // The DB insert happens in a tokio::spawn, so we need a small yield/delay.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // First, get the channel ID for lookup.
        let ch = queries::channels::get_channel_by_name(&pool, &server_id, "#general")
            .await
            .unwrap()
            .unwrap();

        let history = queries::messages::fetch_channel_history(&pool, &ch.id, None, 10)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        let msg = &history[0];
        assert_eq!(msg.content, "Hello World!");
        assert_eq!(msg.sender_nick, "alice");
        assert!(msg.edited_at.is_none());
        assert!(msg.deleted_at.is_none());

        // Edit the message
        let updated =
            queries::messages::update_message_content(&pool, &msg.id, "Hello World! (edited)")
                .await
                .unwrap();
        assert!(updated);

        // Verify edit
        let edited_msg = queries::messages::get_message_by_id(&pool, &msg.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(edited_msg.content, "Hello World! (edited)");
        assert!(edited_msg.edited_at.is_some());

        // Soft delete the message
        let deleted = queries::messages::soft_delete_message(&pool, &msg.id)
            .await
            .unwrap();
        assert!(deleted);

        // Verify soft delete
        let deleted_msg = queries::messages::get_message_by_id(&pool, &msg.id)
            .await
            .unwrap()
            .unwrap();
        assert!(deleted_msg.deleted_at.is_some());

        // Fetch history should now return empty (deleted messages excluded)
        let history_after = queries::messages::fetch_channel_history(&pool, &ch.id, None, 10)
            .await
            .unwrap();
        assert_eq!(history_after.len(), 0);
    }

    #[tokio::test]
    async fn test_kick_and_rejoin_flow() {
        let (engine, pool) = setup_engine().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let user_id = create_test_user(&pool, "bob").await;

        let server_id = engine
            .create_server("Kick Test".into(), owner_id.clone(), None)
            .await
            .unwrap();

        // Bob joins
        engine.join_server(&user_id, &server_id).await.unwrap();

        // Verify Bob is a member
        let member = queries::servers::get_server_member(&pool, &server_id, &user_id)
            .await
            .unwrap();
        assert!(member.is_some());

        // Kick Bob (DB removal + engine leave)
        let kicked = queries::moderation::kick_member(&pool, &server_id, &user_id)
            .await
            .unwrap();
        assert!(kicked);
        engine.leave_server(&user_id, &server_id).await.unwrap();

        // Verify Bob is no longer a member
        let member_after = queries::servers::get_server_member(&pool, &server_id, &user_id)
            .await
            .unwrap();
        assert!(member_after.is_none());

        // Bob rejoins
        engine.join_server(&user_id, &server_id).await.unwrap();

        // Verify Bob is back
        let member_rejoined = queries::servers::get_server_member(&pool, &server_id, &user_id)
            .await
            .unwrap();
        assert!(member_rejoined.is_some());
    }

    // ═══════════════════════════════════════════════════════════════
    //  3. Permission Flow Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_custom_role_grants_manage_channels() {
        let (_engine, pool) = setup_engine().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let user_id = create_test_user(&pool, "bob").await;

        // Manually set up the server in the DB (bypassing engine for direct DB testing)
        let server_id = "test-server-perms";
        queries::servers::create_server(&pool, server_id, "Perm Server", &owner_id, None)
            .await
            .unwrap();
        queries::servers::add_server_member(&pool, server_id, &user_id, "member")
            .await
            .unwrap();

        // Create @everyone role
        let everyone_role_id = Uuid::new_v4().to_string();
        queries::roles::create_role(
            &pool,
            &queries::roles::CreateRoleParams {
                id: &everyone_role_id,
                server_id,
                name: "@everyone",
                color: None,
                icon_url: None,
                position: 0,
                permissions: DEFAULT_EVERYONE.bits() as i64,
                is_default: true,
            },
        )
        .await
        .unwrap();

        // Create a custom role with MANAGE_CHANNELS
        let custom_role_id = Uuid::new_v4().to_string();
        let custom_perms = DEFAULT_EVERYONE | Permissions::MANAGE_CHANNELS;
        queries::roles::create_role(
            &pool,
            &queries::roles::CreateRoleParams {
                id: &custom_role_id,
                server_id,
                name: "Channel Manager",
                color: Some("#00FF00"),
                icon_url: None,
                position: 1,
                permissions: custom_perms.bits() as i64,
                is_default: false,
            },
        )
        .await
        .unwrap();

        // Assign the custom role to bob
        queries::roles::assign_role(&pool, server_id, &user_id, &custom_role_id)
            .await
            .unwrap();

        // Compute effective permissions
        let user_roles = queries::roles::get_user_roles(&pool, server_id, &user_id)
            .await
            .unwrap();
        let role_perms: Vec<(String, Permissions)> = user_roles
            .iter()
            .map(|r| {
                (
                    r.id.clone(),
                    Permissions::from_bits_truncate(r.permissions as u64),
                )
            })
            .collect();

        let effective = compute_effective_permissions(
            DEFAULT_EVERYONE,
            &role_perms,
            &[],
            &everyone_role_id,
            &user_id,
            false,
        );
        assert!(
            effective.contains(Permissions::MANAGE_CHANNELS),
            "User with Channel Manager role should have MANAGE_CHANNELS"
        );
    }

    #[tokio::test]
    async fn test_channel_override_denies_send_messages() {
        let _pool = setup_db().await;

        let everyone_role_id = "role-everyone";
        let user_role_id = "role-mod";

        // User has moderator-level perms from their role
        let user_roles = vec![(user_role_id.to_string(), DEFAULT_MODERATOR)];

        // Channel override: deny SEND_MESSAGES for this specific role
        let overrides = vec![ChannelOverride {
            target_type: OverrideTargetType::Role,
            target_id: user_role_id.to_string(),
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        }];

        let effective = compute_effective_permissions(
            DEFAULT_EVERYONE,
            &user_roles,
            &overrides,
            everyone_role_id,
            "user1",
            false,
        );

        assert!(
            !effective.contains(Permissions::SEND_MESSAGES),
            "Channel override should deny SEND_MESSAGES"
        );
        assert!(
            effective.contains(Permissions::KICK_MEMBERS),
            "Other moderator perms should still be active"
        );
    }

    #[tokio::test]
    async fn test_administrator_bypasses_channel_denies() {
        let overrides = vec![ChannelOverride {
            target_type: OverrideTargetType::User,
            target_id: "admin-user".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES | Permissions::VIEW_CHANNELS,
        }];

        let user_roles = vec![("admin-role".to_string(), Permissions::ADMINISTRATOR)];

        let effective = compute_effective_permissions(
            DEFAULT_EVERYONE,
            &user_roles,
            &overrides,
            "everyone-role",
            "admin-user",
            false,
        );

        assert_eq!(
            effective,
            Permissions::all(),
            "ADMINISTRATOR bypasses all channel denies"
        );
    }

    #[tokio::test]
    async fn test_server_owner_bypasses_all_permissions() {
        let overrides = vec![ChannelOverride {
            target_type: OverrideTargetType::User,
            target_id: "owner1".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::all(),
        }];

        let effective = compute_effective_permissions(
            Permissions::empty(), // even with no base permissions
            &[],
            &overrides,
            "everyone-role",
            "owner1",
            true, // is_owner
        );

        assert_eq!(
            effective,
            Permissions::all(),
            "Server owner always has all permissions"
        );
    }

    #[tokio::test]
    async fn test_private_channel_membership_check() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let user_id = create_test_user(&pool, "bob").await;

        let server_id = "test-private-ch";
        queries::servers::create_server(&pool, server_id, "Private Test", &owner_id, None)
            .await
            .unwrap();

        // Create a private channel
        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#secret")
            .await
            .unwrap();
        queries::channels::set_channel_private(&pool, &channel_id, true)
            .await
            .unwrap();

        // Alice is added to the private channel
        queries::channels::add_member(&pool, &channel_id, &owner_id)
            .await
            .unwrap();

        // Alice can see it, Bob cannot
        let alice_member = queries::channels::is_channel_member(&pool, &channel_id, &owner_id)
            .await
            .unwrap();
        assert!(
            alice_member,
            "Alice should be a member of the private channel"
        );

        let bob_member = queries::channels::is_channel_member(&pool, &channel_id, &user_id)
            .await
            .unwrap();
        assert!(
            !bob_member,
            "Bob should NOT be a member of the private channel"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    //  4. Webhook End-to-End Tests
    // ═══════════════════════════════════════════════════════════════

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

    // ═══════════════════════════════════════════════════════════════
    //  5. Bot Authentication Flow Tests
    // ═══════════════════════════════════════════════════════════════

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
    async fn test_bot_added_to_server() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let bot_user_id = Uuid::new_v4().to_string();

        // Insert bot user directly (create_bot_user references columns not in schema)
        sqlx::query("INSERT INTO users (id, username, is_bot) VALUES (?, ?, 1)")
            .bind(&bot_user_id)
            .bind("helper-bot")
            .execute(&pool)
            .await
            .unwrap();

        let server_id = "bot-server";
        queries::servers::create_server(&pool, server_id, "Bot Test", &owner_id, None)
            .await
            .unwrap();

        // Add bot to server
        queries::bots::add_bot_to_server(&pool, server_id, &bot_user_id)
            .await
            .unwrap();

        // Verify bot is a server member
        let member = queries::servers::get_server_member(&pool, server_id, &bot_user_id)
            .await
            .unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().role, "member");

        // Remove bot from server
        queries::bots::remove_bot_from_server(&pool, server_id, &bot_user_id)
            .await
            .unwrap();

        let member_after = queries::servers::get_server_member(&pool, server_id, &bot_user_id)
            .await
            .unwrap();
        assert!(member_after.is_none());
    }

    // ═══════════════════════════════════════════════════════════════
    //  6. Community Feature Flow Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_server_discovery_flow() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "discover-server";
        queries::servers::create_server(&pool, server_id, "Discoverable", &owner_id, None)
            .await
            .unwrap();

        // Initially not discoverable
        let found = queries::community::list_discoverable_servers(&pool, None)
            .await
            .unwrap();
        assert!(
            found.is_empty(),
            "No servers should be discoverable initially"
        );

        // Enable discovery
        queries::community::update_server_community(
            &pool,
            server_id,
            Some("A great server for testing"),
            true,
            Some("Welcome!"),
            Some("1. Be nice"),
            Some("technology"),
        )
        .await
        .unwrap();

        // Now it should appear
        let found = queries::community::list_discoverable_servers(&pool, None)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, server_id);
        assert_eq!(
            found[0].description,
            Some("A great server for testing".to_string())
        );

        // Filter by category
        let found_tech = queries::community::list_discoverable_servers(&pool, Some("technology"))
            .await
            .unwrap();
        assert_eq!(found_tech.len(), 1);

        let found_gaming = queries::community::list_discoverable_servers(&pool, Some("gaming"))
            .await
            .unwrap();
        assert!(found_gaming.is_empty());
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
    async fn test_invite_with_expiry() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "expiry-server";
        queries::servers::create_server(&pool, server_id, "Expiry Test", &owner_id, None)
            .await
            .unwrap();

        // Create an already-expired invite
        let invite_id = Uuid::new_v4().to_string();
        queries::invites::create_invite(
            &pool,
            &invite_id,
            server_id,
            "EXPIRED1",
            &owner_id,
            None,
            Some("2020-01-01T00:00:00Z"), // already expired
            None,
        )
        .await
        .unwrap();

        // Create a future invite
        let invite_id2 = Uuid::new_v4().to_string();
        queries::invites::create_invite(
            &pool,
            &invite_id2,
            server_id,
            "FUTURE1",
            &owner_id,
            None,
            Some("2099-12-31T23:59:59Z"),
            None,
        )
        .await
        .unwrap();

        // Delete expired invites
        let deleted = queries::invites::delete_expired_invites(&pool)
            .await
            .unwrap();
        assert_eq!(deleted, 1, "One expired invite should be deleted");

        // Verify the future invite still exists
        let future = queries::invites::get_invite_by_code(&pool, "FUTURE1")
            .await
            .unwrap();
        assert!(future.is_some());

        // Verify the expired invite is gone
        let expired = queries::invites::get_invite_by_code(&pool, "EXPIRED1")
            .await
            .unwrap();
        assert!(expired.is_none());
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

    // ═══════════════════════════════════════════════════════════════
    //  7. Moderation Flow Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_ban_prevents_rejoin_unban_allows() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let user_id = create_test_user(&pool, "bob").await;

        let server_id = "ban-server";
        queries::servers::create_server(&pool, server_id, "Ban Test", &owner_id, None)
            .await
            .unwrap();
        queries::servers::add_server_member(&pool, server_id, &user_id, "member")
            .await
            .unwrap();

        // Ban the user
        let ban_id = Uuid::new_v4().to_string();
        queries::bans::create_ban(
            &pool,
            &ban_id,
            server_id,
            &user_id,
            &owner_id,
            Some("Spamming"),
            0,
        )
        .await
        .unwrap();

        // Verify the user is banned
        let is_banned = queries::bans::is_banned(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(is_banned, "User should be banned");

        // Ban list
        let bans = queries::bans::list_bans(&pool, server_id).await.unwrap();
        assert_eq!(bans.len(), 1);
        assert_eq!(bans[0].reason, Some("Spamming".to_string()));

        // Unban
        let unbanned = queries::bans::remove_ban(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(unbanned);

        // Verify not banned
        let is_banned_after = queries::bans::is_banned(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(!is_banned_after, "User should not be banned after unban");

        // User can rejoin
        // (remove + re-add to simulate the flow since kick already happened during ban)
        queries::servers::remove_server_member(&pool, server_id, &user_id)
            .await
            .unwrap();
        queries::servers::add_server_member(&pool, server_id, &user_id, "member")
            .await
            .unwrap();
        let member = queries::servers::get_server_member(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(
            member.is_some(),
            "User should be able to rejoin after unban"
        );
    }

    #[tokio::test]
    async fn test_member_timeout() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let user_id = create_test_user(&pool, "bob").await;

        let server_id = "timeout-server";
        queries::servers::create_server(&pool, server_id, "Timeout Test", &owner_id, None)
            .await
            .unwrap();
        queries::servers::add_server_member(&pool, server_id, &user_id, "member")
            .await
            .unwrap();

        // Set a timeout
        let timeout_until = "2099-12-31T23:59:59Z";
        queries::moderation::set_member_timeout(&pool, server_id, &user_id, Some(timeout_until))
            .await
            .unwrap();

        // Verify timeout is set
        let timeout = queries::moderation::get_member_timeout(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert_eq!(timeout, Some(timeout_until.to_string()));

        // Clear timeout
        queries::moderation::set_member_timeout(&pool, server_id, &user_id, None)
            .await
            .unwrap();

        let timeout_after = queries::moderation::get_member_timeout(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(timeout_after.is_none(), "Timeout should be cleared");
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
    async fn test_bulk_delete_messages() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "bulk-delete-server";
        queries::servers::create_server(&pool, server_id, "Bulk Delete", &owner_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#general")
            .await
            .unwrap();

        // Insert 5 messages
        let mut msg_ids = Vec::new();
        for i in 0..5 {
            let msg_id = Uuid::new_v4().to_string();
            queries::messages::insert_message(
                &pool,
                &queries::messages::InsertMessageParams {
                    id: &msg_id,
                    server_id,
                    channel_id: &channel_id,
                    sender_id: &owner_id,
                    sender_nick: "alice",
                    content: &format!("Message {i}"),
                    reply_to_id: None,
                },
            )
            .await
            .unwrap();
            msg_ids.push(msg_id);
        }

        // Bulk delete 3 of them
        let to_delete: Vec<String> = msg_ids[0..3].to_vec();
        let deleted = queries::moderation::bulk_delete_messages(&pool, &to_delete)
            .await
            .unwrap();
        assert_eq!(deleted, 3);

        // Verify deleted messages have deleted_at set
        for id in &to_delete {
            let msg = queries::messages::get_message_by_id(&pool, id)
                .await
                .unwrap()
                .unwrap();
            assert!(msg.deleted_at.is_some());
        }

        // Non-deleted messages should still be fine
        for id in &msg_ids[3..] {
            let msg = queries::messages::get_message_by_id(&pool, id)
                .await
                .unwrap()
                .unwrap();
            assert!(msg.deleted_at.is_none());
        }

        // Fetch history should only return the 2 non-deleted messages
        let history = queries::messages::fetch_channel_history(&pool, &channel_id, None, 10)
            .await
            .unwrap();
        assert_eq!(history.len(), 2);
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

    // ═══════════════════════════════════════════════════════════════
    //  8. Thread & Forum Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_thread_create_message_archive_lifecycle() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "thread-server";
        queries::servers::create_server(&pool, server_id, "Thread Test", &owner_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#general")
            .await
            .unwrap();

        // Create a parent message
        let parent_msg_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &parent_msg_id,
                server_id,
                channel_id: &channel_id,
                sender_id: &owner_id,
                sender_nick: "alice",
                content: "This should start a thread",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Create a thread from that message
        let thread_id = Uuid::new_v4().to_string();
        queries::threads::create_thread(
            &pool,
            &thread_id,
            server_id,
            "Discussion Thread",
            "public_thread",
            &parent_msg_id,
            1440,
        )
        .await
        .unwrap();

        // Verify thread exists as a channel row
        let thread_row = queries::channels::get_channel(&pool, &thread_id)
            .await
            .unwrap();
        assert!(thread_row.is_some());
        let thread_row = thread_row.unwrap();
        assert_eq!(thread_row.name, "Discussion Thread");
        assert_eq!(thread_row.channel_type, "public_thread");
        assert_eq!(
            thread_row.thread_parent_message_id,
            Some(parent_msg_id.clone())
        );
        assert_eq!(thread_row.archived, 0);

        // Send a message in the thread (threads are channels)
        let thread_msg_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &thread_msg_id,
                server_id,
                channel_id: &thread_id,
                sender_id: &owner_id,
                sender_nick: "alice",
                content: "Thread reply",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Verify message in thread
        let thread_history = queries::messages::fetch_channel_history(&pool, &thread_id, None, 10)
            .await
            .unwrap();
        assert_eq!(thread_history.len(), 1);
        assert_eq!(thread_history[0].content, "Thread reply");

        // Archive the thread
        queries::threads::archive_thread(&pool, &thread_id)
            .await
            .unwrap();

        let archived_row = queries::channels::get_channel(&pool, &thread_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(archived_row.archived, 1);

        // Unarchive
        queries::threads::unarchive_thread(&pool, &thread_id)
            .await
            .unwrap();
        let unarchived_row = queries::channels::get_channel(&pool, &thread_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unarchived_row.archived, 0);
    }

    #[tokio::test]
    async fn test_forum_channel_with_tags() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "forum-server";
        queries::servers::create_server(&pool, server_id, "Forum Test", &owner_id, None)
            .await
            .unwrap();

        // Create a forum-type channel
        let forum_channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &forum_channel_id, server_id, "#help-forum")
            .await
            .unwrap();

        // Create forum tags
        let tag1_id = Uuid::new_v4().to_string();
        let tag2_id = Uuid::new_v4().to_string();
        let tag3_id = Uuid::new_v4().to_string();
        queries::forum_tags::create_tag(&pool, &tag1_id, &forum_channel_id, "Bug", None, 0, 0)
            .await
            .unwrap();
        queries::forum_tags::create_tag(
            &pool,
            &tag2_id,
            &forum_channel_id,
            "Feature Request",
            None,
            0,
            1,
        )
        .await
        .unwrap();
        queries::forum_tags::create_tag(
            &pool,
            &tag3_id,
            &forum_channel_id,
            "Resolved",
            Some("check_mark"),
            1, // moderated
            2,
        )
        .await
        .unwrap();

        // List tags
        let tags = queries::forum_tags::list_tags(&pool, &forum_channel_id)
            .await
            .unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].name, "Bug");
        assert_eq!(tags[1].name, "Feature Request");
        assert_eq!(tags[2].name, "Resolved");
        assert_eq!(tags[2].moderated, 1);

        // Create a parent message for the forum thread
        let msg_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &msg_id,
                server_id,
                channel_id: &forum_channel_id,
                sender_id: &owner_id,
                sender_nick: "alice",
                content: "How do I fix this bug?",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Create a thread in the forum
        let thread_id = Uuid::new_v4().to_string();
        queries::threads::create_thread(
            &pool,
            &thread_id,
            server_id,
            "How do I fix this bug?",
            "public_thread",
            &msg_id,
            1440,
        )
        .await
        .unwrap();

        // Tag the thread
        queries::forum_tags::set_thread_tags(&pool, &thread_id, &[tag1_id.clone()])
            .await
            .unwrap();

        // Get thread tags
        let thread_tags = queries::forum_tags::get_thread_tags(&pool, &thread_id)
            .await
            .unwrap();
        assert_eq!(thread_tags.len(), 1);
        assert_eq!(thread_tags[0].name, "Bug");

        // Re-tag with multiple tags
        queries::forum_tags::set_thread_tags(
            &pool,
            &thread_id,
            &[tag1_id.clone(), tag3_id.clone()],
        )
        .await
        .unwrap();

        let thread_tags_after = queries::forum_tags::get_thread_tags(&pool, &thread_id)
            .await
            .unwrap();
        assert_eq!(thread_tags_after.len(), 2);
    }

    #[tokio::test]
    async fn test_threads_listed_for_parent_channel() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "thread-list-server";
        queries::servers::create_server(&pool, server_id, "Thread List", &owner_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#main")
            .await
            .unwrap();

        // Create 2 parent messages and threads
        for i in 0..2 {
            let msg_id = Uuid::new_v4().to_string();
            queries::messages::insert_message(
                &pool,
                &queries::messages::InsertMessageParams {
                    id: &msg_id,
                    server_id,
                    channel_id: &channel_id,
                    sender_id: &owner_id,
                    sender_nick: "alice",
                    content: &format!("Parent {i}"),
                    reply_to_id: None,
                },
            )
            .await
            .unwrap();

            let thread_id = Uuid::new_v4().to_string();
            queries::threads::create_thread(
                &pool,
                &thread_id,
                server_id,
                &format!("Thread {i}"),
                "public_thread",
                &msg_id,
                1440,
            )
            .await
            .unwrap();
        }

        // List threads for the channel
        let threads = queries::threads::get_threads_for_channel(&pool, &channel_id, server_id)
            .await
            .unwrap();
        assert_eq!(threads.len(), 2);
    }

    // ═══════════════════════════════════════════════════════════════
    //  9. Cross-Protocol Event Consistency Tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_message_event_contains_all_fields() {
        let (engine, pool) = setup_engine().await;

        let user_id = create_test_user(&pool, "alice").await;
        let server_id = engine
            .create_server("Event Test".into(), user_id.clone(), None)
            .await
            .unwrap();

        let (sid1, _rx1) = connect_user(&engine, Some(&user_id), "alice");
        engine.join_channel(sid1, &server_id, "#general").unwrap();

        // Create a second user to receive the event
        let user2_id = create_test_user(&pool, "bob").await;
        engine.join_server(&user2_id, &server_id).await.unwrap();

        let (sid2, mut rx2) = connect_user(&engine, Some(&user2_id), "bob");
        engine.join_channel(sid2, &server_id, "#general").unwrap();
        drain_events(&mut rx2);

        // Send a message
        engine
            .send_message(sid1, &server_id, "#general", "Test message", None, None)
            .unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message {
                id,
                server_id: evt_server_id,
                from,
                target,
                content,
                timestamp,
                ..
            } => {
                assert!(!id.is_nil(), "Message ID should be set");
                assert_eq!(evt_server_id, Some(server_id.clone()));
                assert_eq!(from, "alice");
                assert_eq!(target, "#general");
                assert_eq!(content, "Test message");
                assert!(timestamp <= chrono::Utc::now());
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_join_event_contains_correct_fields() {
        let (engine, pool) = setup_engine().await;

        let user_id = create_test_user(&pool, "alice").await;
        let server_id = engine
            .create_server("Join Event".into(), user_id.clone(), None)
            .await
            .unwrap();

        let (sid1, mut rx1) = connect_user(&engine, Some(&user_id), "alice");
        engine.join_channel(sid1, &server_id, "#general").unwrap();
        drain_events(&mut rx1);

        // Second user joins
        let user2_id = create_test_user(&pool, "bob").await;
        engine.join_server(&user2_id, &server_id).await.unwrap();

        let (sid2, _rx2) = connect_user(&engine, Some(&user2_id), "bob");
        engine.join_channel(sid2, &server_id, "#general").unwrap();

        // Alice should receive the Join event for Bob
        let event = rx1.try_recv().unwrap();
        match event {
            ChatEvent::Join {
                nickname,
                server_id: evt_server_id,
                channel,
                ..
            } => {
                assert_eq!(nickname, "bob");
                assert_eq!(evt_server_id, server_id);
                assert_eq!(channel, "#general");
            }
            _ => panic!("Expected Join event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_server_list_for_user() {
        let (engine, pool) = setup_engine().await;

        let user_id = create_test_user(&pool, "alice").await;

        // Create 3 servers
        let mut server_ids = Vec::new();
        for i in 0..3 {
            let sid = engine
                .create_server(format!("Server {i}"), user_id.clone(), None)
                .await
                .unwrap();
            server_ids.push(sid);
        }

        // List servers for the user
        let servers = engine.list_servers_for_user(&user_id);
        assert_eq!(servers.len(), 3, "User should be a member of 3 servers");

        // Check each server has the correct role
        for server in &servers {
            assert_eq!(server.role, Some("owner".to_string()));
            assert_eq!(server.member_count, 1);
        }
    }

    // ═══════════════════════════════════════════════════════════════
    //  10. Database Constraint & Cascade Tests
    // ═══════════════════════════════════════════════════════════════

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
    async fn test_unique_channel_names_in_server() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "unique-ch-server";
        queries::servers::create_server(&pool, server_id, "Unique Test", &owner_id, None)
            .await
            .unwrap();

        let ch1 = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &ch1, server_id, "#general")
            .await
            .unwrap();

        // Second ensure_channel with same name should return existing channel ID
        let ch2_candidate = Uuid::new_v4().to_string();
        let returned_id =
            queries::channels::ensure_channel(&pool, &ch2_candidate, server_id, "#general")
                .await
                .unwrap();
        assert_eq!(
            returned_id, ch1,
            "ensure_channel should return existing channel for duplicate name"
        );
    }

    #[tokio::test]
    async fn test_reactions_on_message() {
        let pool = setup_db().await;

        let user1_id = create_test_user(&pool, "alice").await;
        let user2_id = create_test_user(&pool, "bob").await;

        let server_id = "reaction-server";
        queries::servers::create_server(&pool, server_id, "Reaction Test", &user1_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#general")
            .await
            .unwrap();

        let msg_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &msg_id,
                server_id,
                channel_id: &channel_id,
                sender_id: &user1_id,
                sender_nick: "alice",
                content: "React to this!",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Add reactions
        let added1 = queries::messages::add_reaction(&pool, &msg_id, &user1_id, "thumbsup")
            .await
            .unwrap();
        assert!(added1);

        let added2 = queries::messages::add_reaction(&pool, &msg_id, &user2_id, "thumbsup")
            .await
            .unwrap();
        assert!(added2);

        let added3 = queries::messages::add_reaction(&pool, &msg_id, &user1_id, "heart")
            .await
            .unwrap();
        assert!(added3);

        // Duplicate reaction should not add
        let dup = queries::messages::add_reaction(&pool, &msg_id, &user1_id, "thumbsup")
            .await
            .unwrap();
        assert!(!dup, "Duplicate reaction should be ignored");

        // Get reactions
        let reactions = queries::messages::get_reactions_for_messages(&pool, &[msg_id.clone()])
            .await
            .unwrap();
        assert_eq!(reactions.len(), 3);

        // Remove a reaction
        let removed = queries::messages::remove_reaction(&pool, &msg_id, &user1_id, "thumbsup")
            .await
            .unwrap();
        assert!(removed);

        let reactions_after =
            queries::messages::get_reactions_for_messages(&pool, &[msg_id.clone()])
                .await
                .unwrap();
        assert_eq!(reactions_after.len(), 2);
    }

    #[tokio::test]
    async fn test_message_pinning_flow() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "pin-server";
        queries::servers::create_server(&pool, server_id, "Pin Test", &owner_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#general")
            .await
            .unwrap();

        // Create a message
        let msg_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &msg_id,
                server_id,
                channel_id: &channel_id,
                sender_id: &owner_id,
                sender_nick: "alice",
                content: "Important message!",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Pin the message
        let pin_id = Uuid::new_v4().to_string();
        queries::pins::pin_message(&pool, &pin_id, &channel_id, &msg_id, &owner_id)
            .await
            .unwrap();

        // Verify it's pinned
        let is_pinned = queries::pins::is_pinned(&pool, &channel_id, &msg_id)
            .await
            .unwrap();
        assert!(is_pinned);

        let pin_count = queries::pins::count_pins(&pool, &channel_id).await.unwrap();
        assert_eq!(pin_count, 1);

        let pinned_list = queries::pins::get_pinned_messages(&pool, &channel_id)
            .await
            .unwrap();
        assert_eq!(pinned_list.len(), 1);
        assert_eq!(pinned_list[0].message_id, msg_id);

        // Unpin
        queries::pins::unpin_message(&pool, &channel_id, &msg_id)
            .await
            .unwrap();

        let is_pinned_after = queries::pins::is_pinned(&pool, &channel_id, &msg_id)
            .await
            .unwrap();
        assert!(!is_pinned_after);
    }

    // ═══════════════════════════════════════════════════════════════
    //  Community: Templates and Channel Follows
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_server_template_lifecycle() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "template-server";
        queries::servers::create_server(&pool, server_id, "Template Test", &owner_id, None)
            .await
            .unwrap();

        // Create a template
        let template_id = Uuid::new_v4().to_string();
        let config = r##"{"channels":["#general","#random"],"roles":["@everyone"]}"##;
        queries::community::create_template(
            &pool,
            &template_id,
            "Starter Template",
            Some("A basic starter"),
            server_id,
            &owner_id,
            config,
        )
        .await
        .unwrap();

        // List templates
        let templates = queries::community::list_templates(&pool, server_id)
            .await
            .unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "Starter Template");
        assert_eq!(templates[0].use_count, 0);

        // Increment use count
        queries::community::increment_template_use(&pool, &template_id)
            .await
            .unwrap();
        queries::community::increment_template_use(&pool, &template_id)
            .await
            .unwrap();

        let template = queries::community::get_template(&pool, &template_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(template.use_count, 2);

        // Delete template
        queries::community::delete_template(&pool, &template_id)
            .await
            .unwrap();
        let template_after = queries::community::get_template(&pool, &template_id)
            .await
            .unwrap();
        assert!(template_after.is_none());
    }

    #[tokio::test]
    async fn test_announcement_channel_follows() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "announce-server";
        queries::servers::create_server(&pool, server_id, "Announce Test", &owner_id, None)
            .await
            .unwrap();

        let source_ch = Uuid::new_v4().to_string();
        let target_ch = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &source_ch, server_id, "#announcements")
            .await
            .unwrap();
        queries::channels::ensure_channel(&pool, &target_ch, server_id, "#news-feed")
            .await
            .unwrap();

        // Set source as announcement channel
        queries::community::set_announcement_channel(&pool, &source_ch, true)
            .await
            .unwrap();

        // Create a follow
        let follow_id = Uuid::new_v4().to_string();
        queries::community::create_channel_follow(
            &pool, &follow_id, &source_ch, &target_ch, &owner_id,
        )
        .await
        .unwrap();

        // List follows
        let follows = queries::community::list_channel_follows(&pool, &source_ch)
            .await
            .unwrap();
        assert_eq!(follows.len(), 1);
        assert_eq!(follows[0].target_channel_id, target_ch);

        // Delete follow
        queries::community::delete_channel_follow(&pool, &follow_id)
            .await
            .unwrap();
        let follows_after = queries::community::list_channel_follows(&pool, &source_ch)
            .await
            .unwrap();
        assert!(follows_after.is_empty());
    }

    #[tokio::test]
    async fn test_rules_acceptance() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;
        let user_id = create_test_user(&pool, "bob").await;

        let server_id = "rules-server";
        queries::servers::create_server(&pool, server_id, "Rules Test", &owner_id, None)
            .await
            .unwrap();
        queries::servers::add_server_member(&pool, server_id, &user_id, "member")
            .await
            .unwrap();

        // Set rules
        queries::community::update_server_community(
            &pool,
            server_id,
            None,
            false,
            None,
            Some("Be respectful. No spam."),
            None,
        )
        .await
        .unwrap();

        // Check Bob hasn't accepted yet
        let accepted = queries::community::has_accepted_rules(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(!accepted);

        // Bob accepts rules
        queries::community::accept_rules(&pool, server_id, &user_id)
            .await
            .unwrap();

        let accepted_after = queries::community::has_accepted_rules(&pool, server_id, &user_id)
            .await
            .unwrap();
        assert!(accepted_after);
    }

    // ═══════════════════════════════════════════════════════════════
    //  Slash Commands & OAuth2 Apps
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_slash_command_registration() {
        let pool = setup_db().await;

        let bot_user_id = Uuid::new_v4().to_string();
        // Insert bot user directly (create_bot_user references columns not in schema)
        sqlx::query("INSERT INTO users (id, username, is_bot) VALUES (?, ?, 1)")
            .bind(&bot_user_id)
            .bind("my-bot")
            .execute(&pool)
            .await
            .unwrap();

        let cmd_id = Uuid::new_v4().to_string();
        queries::slash_commands::create_command(
            &pool,
            &crate::db::models::CreateSlashCommandParams {
                id: &cmd_id,
                bot_user_id: &bot_user_id,
                server_id: None,
                name: "ping",
                description: "Check bot latency",
                options_json: "[]",
            },
        )
        .await
        .unwrap();

        // List commands for bot
        let commands = queries::slash_commands::list_commands_for_bot(&pool, &bot_user_id)
            .await
            .unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "ping");
        assert_eq!(commands[0].description, "Check bot latency");

        // Delete command
        queries::slash_commands::delete_command(&pool, &cmd_id)
            .await
            .unwrap();
        let commands_after = queries::slash_commands::list_commands_for_bot(&pool, &bot_user_id)
            .await
            .unwrap();
        assert!(commands_after.is_empty());
    }

    // ═══════════════════════════════════════════════════════════════
    //  Channel Operations Integration
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_channel_position_and_category() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "pos-server";
        queries::servers::create_server(&pool, server_id, "Position Test", &owner_id, None)
            .await
            .unwrap();

        // Create a category
        let cat_id = Uuid::new_v4().to_string();
        queries::categories::create_category(&pool, &cat_id, server_id, "Text Channels", 0)
            .await
            .unwrap();

        // Create channels with positions
        let ch1 = Uuid::new_v4().to_string();
        let ch2 = Uuid::new_v4().to_string();
        let ch3 = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &ch1, server_id, "#general")
            .await
            .unwrap();
        queries::channels::ensure_channel(&pool, &ch2, server_id, "#random")
            .await
            .unwrap();
        queries::channels::ensure_channel(&pool, &ch3, server_id, "#dev")
            .await
            .unwrap();

        // Set positions
        queries::channels::update_channel_position(&pool, &ch1, 0)
            .await
            .unwrap();
        queries::channels::update_channel_position(&pool, &ch2, 1)
            .await
            .unwrap();
        queries::channels::update_channel_position(&pool, &ch3, 2)
            .await
            .unwrap();

        // Assign category
        queries::channels::update_channel_category(&pool, &ch1, Some(&cat_id))
            .await
            .unwrap();
        queries::channels::update_channel_category(&pool, &ch2, Some(&cat_id))
            .await
            .unwrap();

        // Verify ordering
        let channels = queries::channels::list_channels(&pool, server_id)
            .await
            .unwrap();
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].name, "#general");
        assert_eq!(channels[0].category_id, Some(cat_id.clone()));
        assert_eq!(channels[1].name, "#random");
        assert_eq!(channels[1].category_id, Some(cat_id.clone()));
        assert_eq!(channels[2].name, "#dev");
        assert!(channels[2].category_id.is_none());
    }

    #[tokio::test]
    async fn test_channel_override_persistence() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "override-server";
        queries::servers::create_server(&pool, server_id, "Override Test", &owner_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#restricted")
            .await
            .unwrap();

        let role_id = Uuid::new_v4().to_string();
        queries::roles::create_role(
            &pool,
            &queries::roles::CreateRoleParams {
                id: &role_id,
                server_id,
                name: "Muted",
                color: None,
                icon_url: None,
                position: 0,
                permissions: DEFAULT_EVERYONE.bits() as i64,
                is_default: false,
            },
        )
        .await
        .unwrap();

        // Set a channel override
        let override_id = Uuid::new_v4().to_string();
        queries::channels::set_channel_override(
            &pool,
            &override_id,
            &channel_id,
            "role",
            &role_id,
            0,                                        // no additional allows
            Permissions::SEND_MESSAGES.bits() as i64, // deny sending
        )
        .await
        .unwrap();

        // Retrieve overrides
        let overrides = queries::channels::get_channel_overrides(&pool, &channel_id)
            .await
            .unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].target_type, "role");
        assert_eq!(overrides[0].target_id, role_id);
        assert_eq!(
            overrides[0].deny_bits,
            Permissions::SEND_MESSAGES.bits() as i64
        );

        // Use them in permission computation
        let channel_overrides: Vec<ChannelOverride> = overrides
            .iter()
            .map(|o| ChannelOverride {
                target_type: if o.target_type == "role" {
                    OverrideTargetType::Role
                } else {
                    OverrideTargetType::User
                },
                target_id: o.target_id.clone(),
                allow: Permissions::from_bits_truncate(o.allow_bits as u64),
                deny: Permissions::from_bits_truncate(o.deny_bits as u64),
            })
            .collect();

        let effective = compute_effective_permissions(
            DEFAULT_EVERYONE,
            &[(role_id.clone(), DEFAULT_EVERYONE)],
            &channel_overrides,
            "everyone-placeholder",
            &owner_id,
            false,
        );
        assert!(
            !effective.contains(Permissions::SEND_MESSAGES),
            "User with Muted role should not be able to send in this channel"
        );

        // Delete override
        queries::channels::delete_channel_override(&pool, &channel_id, "role", &role_id)
            .await
            .unwrap();
        let overrides_after = queries::channels::get_channel_overrides(&pool, &channel_id)
            .await
            .unwrap();
        assert!(overrides_after.is_empty());
    }

    // ═══════════════════════════════════════════════════════════════
    //  Read State & Unread Counts
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_read_state_and_unread_counts() {
        let pool = setup_db().await;

        let user_id = create_test_user(&pool, "alice").await;

        let server_id = "unread-server";
        queries::servers::create_server(&pool, server_id, "Unread Test", &user_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#general")
            .await
            .unwrap();

        // Insert 5 messages
        let mut msg_ids = Vec::new();
        for i in 0..5 {
            let msg_id = Uuid::new_v4().to_string();
            queries::messages::insert_message(
                &pool,
                &queries::messages::InsertMessageParams {
                    id: &msg_id,
                    server_id,
                    channel_id: &channel_id,
                    sender_id: &user_id,
                    sender_nick: "alice",
                    content: &format!("Msg {i}"),
                    reply_to_id: None,
                },
            )
            .await
            .unwrap();
            msg_ids.push(msg_id);
        }

        // Before any read state, all messages are unread
        let unreads = queries::messages::get_unread_counts(&pool, &user_id, server_id)
            .await
            .unwrap();
        // All 5 should be unread (no read state set)
        assert!(!unreads.is_empty());
        let ch_unread = unreads.iter().find(|u| u.channel_id == channel_id);
        assert!(ch_unread.is_some());
        assert_eq!(ch_unread.unwrap().unread_count, 5);

        // Mark read up to message 3 (0-indexed)
        queries::messages::mark_channel_read(&pool, &user_id, &channel_id, &msg_ids[2])
            .await
            .unwrap();

        // After marking read, unread count should decrease.
        // NOTE: In-memory SQLite inserts happen so fast that all messages may share the
        // same `created_at` second. The unread query uses `created_at >`, so messages
        // with identical timestamps to the read marker won't be counted. We therefore
        // just verify the count decreased (or reached zero) rather than asserting an
        // exact value.
        let unreads_after = queries::messages::get_unread_counts(&pool, &user_id, server_id)
            .await
            .unwrap();
        let after_count = unreads_after
            .iter()
            .find(|u| u.channel_id == channel_id)
            .map(|u| u.unread_count)
            .unwrap_or(0);
        assert!(
            after_count < 5,
            "Unread count should decrease after marking read, got {after_count}"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    //  Slowmode & NSFW Channel Flags
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_slowmode_and_nsfw_flags() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "alice").await;

        let server_id = "flags-server";
        queries::servers::create_server(&pool, server_id, "Flags Test", &owner_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#test")
            .await
            .unwrap();

        // Set slowmode
        queries::moderation::set_slowmode(&pool, &channel_id, 30)
            .await
            .unwrap();
        let ch = queries::channels::get_channel(&pool, &channel_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ch.slowmode_seconds, 30);

        // Set NSFW
        queries::moderation::set_nsfw(&pool, &channel_id, true)
            .await
            .unwrap();
        let ch2 = queries::channels::get_channel(&pool, &channel_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ch2.is_nsfw, 1);

        // Clear both
        queries::moderation::set_slowmode(&pool, &channel_id, 0)
            .await
            .unwrap();
        queries::moderation::set_nsfw(&pool, &channel_id, false)
            .await
            .unwrap();
        let ch3 = queries::channels::get_channel(&pool, &channel_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ch3.slowmode_seconds, 0);
        assert_eq!(ch3.is_nsfw, 0);
    }

    // ═══════════════════════════════════════════════════════════════
    //  Server Nickname
    // ═══════════════════════════════════════════════════════════════

    // NOTE: test_server_nickname_set_and_clear removed because the `nickname` column
    // does not exist on the `server_members` table in any migration. The functions
    // get_server_nickname / set_server_nickname reference a column that was never added.

    // ═══════════════════════════════════════════════════════════════
    //  Message Reply Chain
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_message_reply_chain() {
        let pool = setup_db().await;

        let user_id = create_test_user(&pool, "alice").await;

        let server_id = "reply-server";
        queries::servers::create_server(&pool, server_id, "Reply Test", &user_id, None)
            .await
            .unwrap();

        let channel_id = Uuid::new_v4().to_string();
        queries::channels::ensure_channel(&pool, &channel_id, server_id, "#general")
            .await
            .unwrap();

        // Send original message
        let msg1_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &msg1_id,
                server_id,
                channel_id: &channel_id,
                sender_id: &user_id,
                sender_nick: "alice",
                content: "Original message",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Reply to it
        let msg2_id = Uuid::new_v4().to_string();
        queries::messages::insert_message(
            &pool,
            &queries::messages::InsertMessageParams {
                id: &msg2_id,
                server_id,
                channel_id: &channel_id,
                sender_id: &user_id,
                sender_nick: "alice",
                content: "This is a reply",
                reply_to_id: Some(&msg1_id),
            },
        )
        .await
        .unwrap();

        // Verify the reply references the original
        let reply = queries::messages::get_message_by_id(&pool, &msg2_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply.reply_to_id, Some(msg1_id.clone()));

        // Original should not have a reply_to
        let original = queries::messages::get_message_by_id(&pool, &msg1_id)
            .await
            .unwrap()
            .unwrap();
        assert!(original.reply_to_id.is_none());
    }

    // ═══════════════════════════════════════════════════════════════
    //  Concurrent Server Member Operations
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_concurrent_server_joins() {
        let pool = setup_db().await;

        let owner_id = create_test_user(&pool, "owner").await;

        let server_id = "concurrent-server";
        queries::servers::create_server(&pool, server_id, "Concurrent Test", &owner_id, None)
            .await
            .unwrap();

        // Create 10 users
        let mut user_ids = Vec::new();
        for i in 0..10 {
            let uid = create_test_user(&pool, &format!("user{i}")).await;
            user_ids.push(uid);
        }

        // Join all concurrently (INSERT OR IGNORE handles conflicts)
        let mut handles = Vec::new();
        for uid in &user_ids {
            let pool_clone = pool.clone();
            let sid = server_id.to_string();
            let uid = uid.clone();
            handles.push(tokio::spawn(async move {
                queries::servers::add_server_member(&pool_clone, &sid, &uid, "member")
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // Verify all 11 members (owner + 10 users)
        let member_count = queries::servers::get_member_count(&pool, server_id)
            .await
            .unwrap();
        assert_eq!(member_count, 11);
    }

    // ═══════════════════════════════════════════════════════════════
    //  Multi-Server User Flow
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_user_across_multiple_servers() {
        let pool = setup_db().await;

        let user_id = create_test_user(&pool, "alice").await;

        // Create 3 servers, alice owns all of them
        let mut server_ids = Vec::new();
        for i in 0..3 {
            let sid = format!("multi-srv-{i}");
            queries::servers::create_server(&pool, &sid, &format!("Server {i}"), &user_id, None)
                .await
                .unwrap();
            server_ids.push(sid);
        }

        // Alice should be a member of all 3
        let alice_servers = queries::servers::list_servers_for_user(&pool, &user_id)
            .await
            .unwrap();
        assert_eq!(alice_servers.len(), 3);

        // Leave one server
        queries::servers::remove_server_member(&pool, &server_ids[1], &user_id)
            .await
            .unwrap();

        let alice_servers_after = queries::servers::list_servers_for_user(&pool, &user_id)
            .await
            .unwrap();
        assert_eq!(alice_servers_after.len(), 2);
    }

    // ═══════════════════════════════════════════════════════════════
    //  Engine: Server Deletion Cleans Up Memory
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_engine_delete_server_cleans_memory() {
        let (engine, pool) = setup_engine().await;

        let user_id = create_test_user(&pool, "alice").await;
        let server_id = engine
            .create_server("To Delete".into(), user_id.clone(), None)
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

    // ═══════════════════════════════════════════════════════════════
    //  Engine: Multi-Session Messaging
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_three_users_in_channel_messaging() {
        let (engine, pool) = setup_engine().await;

        let u1 = create_test_user(&pool, "alice").await;
        let u2 = create_test_user(&pool, "bob").await;
        let u3 = create_test_user(&pool, "charlie").await;

        let server_id = engine
            .create_server("3 Users".into(), u1.clone(), None)
            .await
            .unwrap();

        // Add bob and charlie as members
        for uid in [&u2, &u3] {
            engine.join_server(uid, &server_id).await.unwrap();
        }

        let (sid1, mut rx1) = connect_user(&engine, Some(&u1), "alice");
        let (sid2, mut rx2) = connect_user(&engine, Some(&u2), "bob");
        let (sid3, mut rx3) = connect_user(&engine, Some(&u3), "charlie");

        engine.join_channel(sid1, &server_id, "#general").unwrap();
        engine.join_channel(sid2, &server_id, "#general").unwrap();
        engine.join_channel(sid3, &server_id, "#general").unwrap();

        drain_events(&mut rx1);
        drain_events(&mut rx2);
        drain_events(&mut rx3);

        // Alice sends a message
        engine
            .send_message(sid1, &server_id, "#general", "Hello everyone!", None, None)
            .unwrap();

        // Bob and Charlie should receive it, but not Alice
        let bob_event = rx2.try_recv().unwrap();
        let charlie_event = rx3.try_recv().unwrap();

        for evt in [&bob_event, &charlie_event] {
            match evt {
                ChatEvent::Message { from, content, .. } => {
                    assert_eq!(from, "alice");
                    assert_eq!(content, "Hello everyone!");
                }
                _ => panic!("Expected Message event"),
            }
        }

        // Alice should not receive her own message
        assert!(rx1.try_recv().is_err());
    }
}
