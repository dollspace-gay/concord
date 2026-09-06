use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_message_send_edit_delete_lifecycle() {
    let (engine, pool) = setup_engine().await;

    let user_id = create_test_user(&pool, "alice").await;
    let server_id = engine
        .create_server_for_actor(
            &actor_for(&pool, &user_id).await,
            "Msg Test Server".into(),
            None,
        )
        .await
        .unwrap();

    // Connect user and join #general
    let (sid, mut rx) = connect_user(&engine, Some(&user_id), "alice");
    authenticate_session(&engine, &pool, &user_id, sid).await;
    engine
        .join_channel(sid, &server_id, "#general")
        .await
        .unwrap();
    drain_events(&mut rx);

    // Send a message
    engine
        .send_message(
            sid,
            &server_id,
            "#general",
            "Hello World!",
            None,
            None,
            None,
        )
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
        &crate::db::queries::threads::CreateThreadParams {
            channel_id: &thread_id,
            server_id,
            name: "Discussion Thread",
            channel_type: "public_thread",
            parent_message_id: &parent_msg_id,
            parent_channel_id: &channel_id,
            creator_user_id: &owner_id,
            auto_archive_minutes: 1440,
        },
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_message_event_contains_all_fields() {
    let (engine, pool) = setup_engine().await;

    let user_id = create_test_user(&pool, "alice").await;
    let server_id = engine
        .create_server_for_actor(&actor_for(&pool, &user_id).await, "Event Test".into(), None)
        .await
        .unwrap();

    let (sid1, _rx1) = connect_user(&engine, Some(&user_id), "alice");
    authenticate_session(&engine, &pool, &user_id, sid1).await;
    engine
        .join_channel(sid1, &server_id, "#general")
        .await
        .unwrap();

    // Create a second user to receive the event
    let user2_id = create_test_user(&pool, "bob").await;
    engine.join_server(&user2_id, &server_id).await.unwrap();

    let (sid2, mut rx2) = connect_user(&engine, Some(&user2_id), "bob");
    authenticate_session(&engine, &pool, &user2_id, sid2).await;
    engine
        .join_channel(sid2, &server_id, "#general")
        .await
        .unwrap();
    drain_events(&mut rx2);

    // Send a message
    engine
        .send_message(
            sid1,
            &server_id,
            "#general",
            "Test message",
            None,
            None,
            None,
        )
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
            assert!(!id.as_str().is_empty(), "Message ID should be set");
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
    let reactions =
        queries::messages::get_reactions_for_messages(&pool, std::slice::from_ref(&msg_id))
            .await
            .unwrap();
    assert_eq!(reactions.len(), 3);

    // Remove a reaction
    let removed = queries::messages::remove_reaction(&pool, &msg_id, &user1_id, "thumbsup")
        .await
        .unwrap();
    assert!(removed);

    let reactions_after =
        queries::messages::get_reactions_for_messages(&pool, std::slice::from_ref(&msg_id))
            .await
            .unwrap();
    assert_eq!(reactions_after.len(), 2);
}
