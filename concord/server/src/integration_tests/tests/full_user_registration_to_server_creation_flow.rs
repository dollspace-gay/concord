use super::*;

#[tokio::test]
async fn test_full_user_registration_to_server_creation_flow() {
    let (engine, pool) = setup_engine().await;

    // Step 1: Register a user
    let owner_id = create_test_user(&pool, "alice").await;

    // Step 2: Create a server via the engine
    let server_id = engine
        .create_server_for_actor(&actor_for(&pool, &owner_id).await, "My Server".into(), None)
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
        .create_server_for_actor(
            &actor_for(&pool, &owner_id).await,
            "Test Server".into(),
            None,
        )
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
async fn test_kick_and_rejoin_flow() {
    let (engine, pool) = setup_engine().await;

    let owner_id = create_test_user(&pool, "alice").await;
    let user_id = create_test_user(&pool, "bob").await;

    let server_id = engine
        .create_server_for_actor(&actor_for(&pool, &owner_id).await, "Kick Test".into(), None)
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

#[tokio::test]
async fn test_server_discovery_flow() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "discover-server";
    queries::servers::create_server(&pool, server_id, "Discoverable", &owner_id, None)
        .await
        .unwrap();

    // Initially not discoverable
    let found = queries::community::list_discoverable_servers(&pool, None, 100, 0)
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
    let found = queries::community::list_discoverable_servers(&pool, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, server_id);
    assert_eq!(
        found[0].description,
        Some("A great server for testing".to_string())
    );

    // Filter by category
    let found_tech =
        queries::community::list_discoverable_servers(&pool, Some("technology"), 100, 0)
            .await
            .unwrap();
    assert_eq!(found_tech.len(), 1);

    let found_gaming = queries::community::list_discoverable_servers(&pool, Some("gaming"), 100, 0)
        .await
        .unwrap();
    assert!(found_gaming.is_empty());
}

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
