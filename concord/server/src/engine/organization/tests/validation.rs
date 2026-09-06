use super::*;

#[tokio::test]
async fn reorder_is_complete_and_role_deletion_invalidates_overrides() {
    let (pool, service, owner, _) = fixture().await;
    service
        .create_channel(
            &owner,
            CreateChannel {
                server_id: &server_id("server"),
                channel_id: &channel_id("one"),
                name: "#one",
                category_id: None,
                is_private: false,
                channel_type: "text",
            },
        )
        .await
        .unwrap();
    service
        .create_channel(
            &owner,
            CreateChannel {
                server_id: &server_id("server"),
                channel_id: &channel_id("two"),
                name: "#two",
                category_id: None,
                is_private: false,
                channel_type: "text",
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        service
            .reorder_channels(
                &owner,
                &server_id("server"),
                &[ChannelPositionInfo {
                    id: "one".into(),
                    position: 0,
                    category_id: None,
                }],
            )
            .await,
        Err(OrganizationError::InvalidInput(_))
    ));
    service
        .reorder_channels(
            &owner,
            &server_id("server"),
            &[
                ChannelPositionInfo {
                    id: "one".into(),
                    position: 1,
                    category_id: None,
                },
                ChannelPositionInfo {
                    id: "two".into(),
                    position: 0,
                    category_id: None,
                },
            ],
        )
        .await
        .unwrap();

    let target = service
        .create_role(
            &owner,
            &server_id("server"),
            "target",
            "Target",
            Some("#123ABC"),
            0,
        )
        .await
        .unwrap();
    let manager_permissions =
        (Permissions::VIEW_CHANNELS | Permissions::MANAGE_ROLES).bits() as i64;
    service
        .create_role(
            &owner,
            &server_id("server"),
            "manager-role",
            "Manager",
            Some("#ABCDEF"),
            manager_permissions,
        )
        .await
        .unwrap();
    service
        .set_member_role(&owner, &server_id("server"), "member", "manager-role", true)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits,deny_bits) VALUES('override','one','role','target',1,0)")
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, manager) = auth.issue_web_session("member").await.unwrap();
    let manager_service = OrganizationService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );
    assert!(matches!(
        manager_service
            .update_role(
                &manager,
                &server_id("server"),
                &target.id,
                "Target",
                Some("#123ABC"),
                Permissions::ADMINISTRATOR.bits() as i64,
            )
            .await,
        Err(OrganizationError::Forbidden)
    ));
    manager_service
        .delete_role(&manager, &server_id("server"), &target.id)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM channel_permission_overrides WHERE target_id='target'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
}
