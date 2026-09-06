use super::*;

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
        &crate::db::queries::threads::CreateThreadParams {
            channel_id: &thread_id,
            server_id,
            name: "How do I fix this bug?",
            channel_type: "public_thread",
            parent_message_id: &msg_id,
            parent_channel_id: &forum_channel_id,
            creator_user_id: &owner_id,
            auto_archive_minutes: 1440,
        },
    )
    .await
    .unwrap();

    // Tag the thread
    queries::forum_tags::set_thread_tags(&pool, &thread_id, std::slice::from_ref(&tag1_id))
        .await
        .unwrap();

    // Get thread tags
    let thread_tags = queries::forum_tags::get_thread_tags(&pool, &thread_id)
        .await
        .unwrap();
    assert_eq!(thread_tags.len(), 1);
    assert_eq!(thread_tags[0].name, "Bug");

    // Re-tag with multiple tags
    queries::forum_tags::set_thread_tags(&pool, &thread_id, &[tag1_id.clone(), tag3_id.clone()])
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
            &crate::db::queries::threads::CreateThreadParams {
                channel_id: &thread_id,
                server_id,
                name: &format!("Thread {i}"),
                channel_type: "public_thread",
                parent_message_id: &msg_id,
                parent_channel_id: &channel_id,
                creator_user_id: &owner_id,
                auto_archive_minutes: 1440,
            },
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

#[tokio::test]
async fn test_join_event_contains_correct_fields() {
    let (engine, pool) = setup_engine().await;

    let user_id = create_test_user(&pool, "alice").await;
    let server_id = engine
        .create_server_for_actor(&actor_for(&pool, &user_id).await, "Join Event".into(), None)
        .await
        .unwrap();

    let (sid1, mut rx1) = connect_user(&engine, Some(&user_id), "alice");
    authenticate_session(&engine, &pool, &user_id, sid1).await;
    engine
        .join_channel(sid1, &server_id, "#general")
        .await
        .unwrap();
    drain_events(&mut rx1);

    // Second user joins
    let user2_id = create_test_user(&pool, "bob").await;
    engine.join_server(&user2_id, &server_id).await.unwrap();

    let (sid2, _rx2) = connect_user(&engine, Some(&user2_id), "bob");
    authenticate_session(&engine, &pool, &user2_id, sid2).await;
    engine
        .join_channel(sid2, &server_id, "#general")
        .await
        .unwrap();

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
    queries::community::create_channel_follow(&pool, &follow_id, &source_ch, &target_ch, &owner_id)
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
