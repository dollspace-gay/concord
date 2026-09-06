use super::*;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_three_users_in_channel_messaging() {
    let (engine, pool) = setup_engine().await;

    let u1 = create_test_user(&pool, "alice").await;
    let u2 = create_test_user(&pool, "bob").await;
    let u3 = create_test_user(&pool, "charlie").await;

    let server_id = engine
        .create_server_for_actor(&actor_for(&pool, &u1).await, "3 Users".into(), None)
        .await
        .unwrap();

    // Add bob and charlie as members
    for uid in [&u2, &u3] {
        engine.join_server(uid, &server_id).await.unwrap();
    }

    let (sid1, mut rx1) = connect_user(&engine, Some(&u1), "alice");
    let (sid2, mut rx2) = connect_user(&engine, Some(&u2), "bob");
    let (sid3, mut rx3) = connect_user(&engine, Some(&u3), "charlie");
    authenticate_session(&engine, &pool, &u1, sid1).await;
    authenticate_session(&engine, &pool, &u2, sid2).await;
    authenticate_session(&engine, &pool, &u3, sid3).await;

    engine
        .join_channel(sid1, &server_id, "#general")
        .await
        .unwrap();
    engine
        .join_channel(sid2, &server_id, "#general")
        .await
        .unwrap();
    engine
        .join_channel(sid3, &server_id, "#general")
        .await
        .unwrap();

    drain_events(&mut rx1);
    drain_events(&mut rx2);
    drain_events(&mut rx3);

    // Alice sends a message
    engine
        .send_message(
            sid1,
            &server_id,
            "#general",
            "Hello everyone!",
            None,
            None,
            None,
        )
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

    // Alice should not receive her own message (only a MessageAck)
    let ack = rx1.try_recv().unwrap();
    assert!(matches!(ack, ChatEvent::MessageAck { .. }));
    assert!(rx1.try_recv().is_err());
}
