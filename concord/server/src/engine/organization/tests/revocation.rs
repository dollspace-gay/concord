use super::*;

#[tokio::test]
async fn revoked_actor_cannot_provision_or_delete_after_admission() {
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
        auth.clone(),
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );
    service
        .provision_server(
            &actor,
            "Existing",
            None,
            &server_id("existing"),
            &channel_id("general"),
            "existing",
        )
        .await
        .unwrap();
    sqlx::query("UPDATE users SET is_system_admin=1 WHERE id='owner'")
        .execute(&pool)
        .await
        .unwrap();
    auth.revoke_credential(actor.credential_id()).await.unwrap();

    assert!(matches!(
        service
            .provision_server(
                &actor,
                "Denied",
                None,
                &server_id("denied"),
                &channel_id("denied-general"),
                "denied"
            )
            .await,
        Err(OrganizationError::Authentication(_))
    ));
    assert!(matches!(
        service
            .delete_owned_server(&actor, &server_id("existing"))
            .await,
        Err(OrganizationError::Authentication(_))
    ));
    assert!(matches!(
        service
            .update_server(&actor, &server_id("existing"), Some("Denied rename"), None)
            .await,
        Err(OrganizationError::Authentication(_))
    ));
    assert!(matches!(
        service
            .admin_delete_server(&actor, &server_id("existing"))
            .await,
        Err(OrganizationError::Authentication(_))
    ));
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM servers WHERE id='existing'), \
                (SELECT count(*) FROM servers WHERE id='denied')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 0));
}
