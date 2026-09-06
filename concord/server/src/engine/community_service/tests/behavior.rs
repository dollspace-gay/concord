use super::*;

#[tokio::test]
async fn competing_announcement_follows_cannot_commit_a_cycle() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('a','A','owner'),('b','B','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('a','owner','owner'),('b','owner','owner')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name,is_announcement) VALUES('ca','a','#a',1),('cb','b','#b',1)").execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );
    let ca = channel_id("ca");
    let cb = channel_id("cb");
    let (left, right) = tokio::join!(
        service.follow_channel(&actor, &ca, &cb),
        service.follow_channel(&actor, &cb, &ca)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_follows")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn scheduled_event_status_transitions_are_monotonic() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO server_events(id,server_id,name,start_time,created_by,integrity_state) VALUES('event','server','Event','2030-01-01T00:00:00Z','owner','active')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool),
    );

    assert!(matches!(
        service
            .update_event_status(&actor, &server_id("server"), "event", "completed")
            .await,
        Err(CommunityError::Forbidden)
    ));
    assert_eq!(
        service
            .update_event_status(&actor, &server_id("server"), "event", "active")
            .await
            .unwrap()
            .status,
        "active"
    );
    assert!(matches!(
        service
            .update_event_status(&actor, &server_id("server"), "event", "scheduled")
            .await,
        Err(CommunityError::Forbidden)
    ));
    assert_eq!(
        service
            .update_event_status(&actor, &server_id("server"), "event", "completed")
            .await
            .unwrap()
            .status,
        "completed"
    );
}
