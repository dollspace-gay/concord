use super::*;

#[tokio::test]
async fn final_invite_use_is_single_winner_and_failed_membership_rolls_back() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('first','first'),('second','second'),('broken','broken')")
        .execute(&pool).await.unwrap();
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
    sqlx::query("INSERT INTO invites(id,server_id,code,created_by,max_uses) VALUES('last','server','last-code','owner',1),('broken-invite','server','broken-code','owner',1)")
        .execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, first) = auth.issue_web_session("first").await.unwrap();
    let (_, second) = auth.issue_web_session("second").await.unwrap();
    let (_, broken) = auth.issue_web_session("broken").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );

    let (first_result, second_result) = tokio::join!(
        service.redeem_invite(&first, "last-code"),
        service.redeem_invite(&second, "last-code"),
    );
    assert_eq!(
        usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
        1
    );
    let use_count: i64 = sqlx::query_scalar("SELECT use_count FROM invites WHERE id='last'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM server_members WHERE server_id='server' AND user_id IN ('first','second')").fetch_one(&pool).await.unwrap();
    assert_eq!((use_count, members), (1, 1));

    sqlx::query("CREATE TRIGGER reject_broken_member BEFORE INSERT ON server_members WHEN NEW.user_id='broken' BEGIN SELECT RAISE(ABORT,'fixture membership failure'); END")
        .execute(&pool).await.unwrap();
    assert!(matches!(
        service.redeem_invite(&broken, "broken-code").await,
        Err(CommunityError::Database(_))
    ));
    let use_count: i64 =
        sqlx::query_scalar("SELECT use_count FROM invites WHERE id='broken-invite'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(use_count, 0);
}
