use super::*;

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
