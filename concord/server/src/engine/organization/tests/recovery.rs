use super::*;

#[tokio::test]
async fn server_provisioning_is_atomic_and_selects_a_real_default_channel() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = OrganizationService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );
    service
        .provision_server(
            &actor,
            "Created",
            None,
            &server_id("server"),
            &channel_id("general"),
            "created-server",
        )
        .await
        .unwrap();
    let defaults: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM servers WHERE id='server'), \
            (SELECT count(*) FROM server_members WHERE server_id='server' AND user_id='owner'), \
            (SELECT count(*) FROM roles WHERE server_id='server' AND is_default=1), \
            (SELECT count(*) FROM channels WHERE server_id='server' AND is_default=1), \
            (SELECT count(*) FROM channel_aliases WHERE server_id='server' AND alias='general')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(defaults, (1, 1, 1, 1, 1));

    assert!(
        service
            .provision_server(
                &actor,
                "Broken",
                None,
                &server_id("broken"),
                &channel_id("broken-channel"),
                "created-server",
            )
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM servers WHERE id='broken'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
