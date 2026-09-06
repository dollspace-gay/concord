use super::*;

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
