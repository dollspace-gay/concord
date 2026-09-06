use super::*;

#[tokio::test]
async fn role_projection_version_advances_only_for_committed_changes() {
    let (pool, service, actor, _) = fixture().await;
    service
        .create_role(
            &actor,
            &server_id("server"),
            "colored",
            "Colored",
            Some("#123456"),
            0,
        )
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT role_projection_version FROM servers WHERE id='server'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    service
        .set_member_role(&actor, &server_id("server"), "member", "colored", true)
        .await
        .unwrap();
    service
        .set_member_role(&actor, &server_id("server"), "member", "colored", true)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT role_projection_version FROM servers WHERE id='server'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        2
    );
    service
        .delete_role(&actor, &server_id("server"), "colored")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT role_projection_version FROM servers WHERE id='server'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        3
    );
}
