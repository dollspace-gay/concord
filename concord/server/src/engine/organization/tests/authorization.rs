use super::*;

#[tokio::test]
async fn default_role_permissions_can_change_without_structural_mutation() {
    let (_pool, service, actor, default_role) = fixture().await;
    let requested = (Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY).bits() as i64;
    let updated = service
        .update_role(
            &actor,
            &server_id("server"),
            &default_role,
            "@everyone",
            None,
            requested,
        )
        .await
        .unwrap();
    assert_eq!(updated.permissions, requested);
    assert!(
        service
            .update_role(
                &actor,
                &server_id("server"),
                &default_role,
                "renamed",
                None,
                requested
            )
            .await
            .is_err()
    );
    assert!(
        service
            .delete_role(&actor, &server_id("server"), &default_role)
            .await
            .is_err()
    );
    assert!(
        service
            .set_member_role(&actor, &server_id("server"), "member", &default_role, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn built_in_role_and_member_avatar_mutations_are_actor_scoped() {
    let (pool, service, owner, _) = fixture().await;
    service
        .update_member_role(&owner, &server_id("server"), "member", "moderator")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT role FROM server_members WHERE server_id='server' AND user_id='member'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "moderator"
    );
    assert!(matches!(
        service
            .update_member_role(&owner, &server_id("server"), "owner", "member")
            .await,
        Err(OrganizationError::Forbidden)
    ));
    service
        .set_member_avatar(
            &owner,
            &server_id("server"),
            Some("https://cdn.test/avatar.png"),
        )
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT avatar_url FROM server_members WHERE server_id='server' AND user_id='owner'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .as_deref(),
        Some("https://cdn.test/avatar.png")
    );
}

#[tokio::test]
async fn channel_overrides_are_scoped_validated_and_reversible() {
    let (pool, service, owner, default_role) = fixture().await;
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')")
        .execute(&pool)
        .await
        .unwrap();

    service
        .set_channel_override(
            &owner,
            ChannelOverrideUpdate {
                server_id: &server_id("server"),
                channel_id: &channel_id("channel"),
                target_type: "role",
                target_id: &default_role,
                allow_bits: Permissions::SEND_MESSAGES.bits() as i64,
                deny_bits: Permissions::ATTACH_FILES.bits() as i64,
            },
        )
        .await
        .unwrap();
    let listed = service
        .list_channel_overrides(&owner, &server_id("server"), &channel_id("channel"))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].target_id, default_role);
    assert_eq!(
        listed[0].allow_bits,
        Permissions::SEND_MESSAGES.bits() as i64
    );
    assert_eq!(listed[0].deny_bits, Permissions::ATTACH_FILES.bits() as i64);

    assert!(matches!(
        service
            .set_channel_override(
                &owner,
                ChannelOverrideUpdate {
                    server_id: &server_id("server"),
                    channel_id: &channel_id("channel"),
                    target_type: "role",
                    target_id: "missing-role",
                    allow_bits: Permissions::SEND_MESSAGES.bits() as i64,
                    deny_bits: 0,
                },
            )
            .await,
        Err(OrganizationError::Forbidden)
    ));
    assert!(matches!(
        service
            .set_channel_override(
                &owner,
                ChannelOverrideUpdate {
                    server_id: &server_id("server"),
                    channel_id: &channel_id("channel"),
                    target_type: "role",
                    target_id: &default_role,
                    allow_bits: Permissions::ADMINISTRATOR.bits() as i64,
                    deny_bits: 0,
                },
            )
            .await,
        Err(OrganizationError::InvalidInput(_))
    ));

    service
        .delete_channel_override(
            &owner,
            &server_id("server"),
            &channel_id("channel"),
            "role",
            &default_role,
        )
        .await
        .unwrap();
    assert!(
        service
            .list_channel_overrides(&owner, &server_id("server"), &channel_id("channel"))
            .await
            .unwrap()
            .is_empty()
    );
}
