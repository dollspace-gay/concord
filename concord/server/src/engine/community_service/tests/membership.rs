use super::*;

#[tokio::test]
async fn source_channel_override_can_deny_announcement_follow() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('manager','manager')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('source','Source','owner'),('target','Target','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('source','manager','member'),('target','manager','member')")
        .execute(&pool)
        .await
        .unwrap();
    let manager_permissions =
        (Permissions::MANAGE_MESSAGES | Permissions::MANAGE_CHANNELS).bits() as i64;
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions) VALUES('source-manager','source','Manager',?),('target-manager','target','Manager',?)")
        .bind(manager_permissions)
        .bind(manager_permissions)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_roles(user_id,server_id,role_id) VALUES('manager','source','source-manager'),('manager','target','target-manager')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name,is_announcement) VALUES('announcement','source','#announcement',1),('destination','target','#destination',0)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('deny-manager','announcement','role','source-manager',?)")
        .bind(Permissions::MANAGE_MESSAGES.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("manager").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );

    assert!(matches!(
        service
            .follow_channel(
                &actor,
                &channel_id("announcement"),
                &channel_id("destination"),
            )
            .await,
        Err(CommunityError::Forbidden)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_follows")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn banned_existing_member_cannot_redeem_invite_idempotently() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO invites(id,server_id,code,created_by) VALUES('invite','server','code','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','member','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("member").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );

    assert!(matches!(
        service.redeem_invite(&actor, "code").await,
        Err(CommunityError::Forbidden)
    ));
    let uses: i64 = sqlx::query_scalar("SELECT use_count FROM invites WHERE id='invite'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 0);
}

#[tokio::test]
async fn rules_acceptance_tracks_the_current_version_and_requires_membership() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users(id,username) VALUES \
         ('owner','owner'),('member','member'),('outsider','outsider')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO servers(id,name,owner_id,rules_text) VALUES('server','Server','owner','v1')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, member) = auth.issue_web_session("member").await.unwrap();
    let (_, outsider) = auth.issue_web_session("outsider").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );

    assert_eq!(
        service
            .accept_rules(&member, &server_id("server"))
            .await
            .unwrap(),
        1
    );
    let (_, accepted, _) = service
        .get_community(&member, &server_id("server"))
        .await
        .unwrap();
    assert!(accepted);
    assert!(
        crate::db::queries::community::has_accepted_rules(&pool, "server", "member")
            .await
            .unwrap()
    );

    sqlx::query("UPDATE servers SET rules_text='v2' WHERE id='server'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !crate::db::queries::community::has_accepted_rules(&pool, "server", "member")
            .await
            .unwrap()
    );
    let (_, accepted, _) = service
        .get_community(&member, &server_id("server"))
        .await
        .unwrap();
    assert!(!accepted);
    assert_eq!(
        service
            .accept_rules(&member, &server_id("server"))
            .await
            .unwrap(),
        2
    );
    assert!(matches!(
        service.accept_rules(&outsider, &server_id("server")).await,
        Err(CommunityError::Forbidden)
    ));
}
