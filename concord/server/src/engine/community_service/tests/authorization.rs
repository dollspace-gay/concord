use super::*;

#[tokio::test]
async fn invite_deletion_is_server_scoped_and_revalidates_actor() {
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
    sqlx::query("INSERT INTO invites(id,server_id,code,created_by) VALUES('invite-a','a','code-a','owner'),('invite-b','b','code-b','owner')").execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth.clone(),
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );
    assert!(matches!(
        service
            .delete_invite(&actor, &server_id("a"), "invite-b")
            .await,
        Err(CommunityError::Forbidden)
    ));
    auth.revoke_credential(actor.credential_id()).await.unwrap();
    assert!(matches!(
        service
            .delete_invite(&actor, &server_id("a"), "invite-a")
            .await,
        Err(CommunityError::Authentication(_))
    ));
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM invites WHERE id IN ('invite-a','invite-b')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 2);
}

#[tokio::test]
async fn vanity_code_is_scoped_unique_audited_and_revalidates_actor() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('other','other')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO servers(id,name,owner_id,vanity_code) VALUES \
         ('a','A','owner',NULL),('b','B','other','taken')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('a','owner','owner'),('b','other','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth.clone(),
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );

    assert!(matches!(
        service
            .set_vanity_code(&actor, &server_id("b"), Some("unavailable"))
            .await,
        Err(CommunityError::Forbidden)
    ));
    assert!(matches!(
        service
            .set_vanity_code(&actor, &server_id("a"), Some("taken"))
            .await,
        Err(CommunityError::Conflict("vanity code unavailable"))
    ));
    service
        .set_vanity_code(&actor, &server_id("a"), Some("available"))
        .await
        .unwrap();
    let persisted: (Option<String>, i64) = sqlx::query_as(
        "SELECT vanity_code,(SELECT count(*) FROM audit_log \
         WHERE server_id='a' AND actor_id='owner' \
         AND action_type='server_vanity_update') FROM servers WHERE id='a'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted, (Some("available".to_owned()), 1));

    auth.revoke_credential(actor.credential_id()).await.unwrap();
    assert!(matches!(
        service
            .set_vanity_code(&actor, &server_id("a"), Some("changed"))
            .await,
        Err(CommunityError::Authentication(_))
    ));
    let persisted: Option<String> =
        sqlx::query_scalar("SELECT vanity_code FROM servers WHERE id='a'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(persisted.as_deref(), Some("available"));
}

#[tokio::test]
async fn templates_are_admin_scoped_coherent_snapshots_and_never_copy_user_grants() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users(id,username) VALUES \
         ('owner','owner'),('manager','manager'),('member','member'),('outsider','outsider')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('source','Source','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('source','owner','owner'),('source','manager','member'),('source','member','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO roles(id,server_id,name,position,permissions,is_default) VALUES \
         ('everyone','source','@everyone',0,?,1), \
         ('private-role','source','Private',1,0,0), \
         ('manager-role','source','Manager',2,?,0)",
    )
    .bind(Permissions::VIEW_CHANNELS.bits() as i64)
    .bind((Permissions::VIEW_CHANNELS | Permissions::MANAGE_SERVER).bits() as i64)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_roles(server_id,user_id,role_id) \
         VALUES('source','manager','manager-role')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channels(id,server_id,name,topic,is_default,position) VALUES \
         ('general','source','#general','Public',1,0), \
         ('secret','source','#secret','Hidden topic',0,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE channels SET is_private=1 WHERE id='secret'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT OR IGNORE INTO channel_aliases(server_id,alias,channel_id) VALUES \
         ('source','home','general'),('source','vault','secret')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channel_permission_overrides( \
            id,channel_id,target_type,target_id,allow_bits,deny_bits \
         ) VALUES \
         ('role-override','secret','role','private-role',?,0), \
         ('manager-deny','secret','role','manager-role',0,?), \
         ('user-override','secret','user','member',?,0)",
    )
    .bind(Permissions::VIEW_CHANNELS.bits() as i64)
    .bind(Permissions::VIEW_CHANNELS.bits() as i64)
    .bind(Permissions::VIEW_CHANNELS.bits() as i64)
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, owner) = auth.issue_web_session("owner").await.unwrap();
    let (_, manager) = auth.issue_web_session("manager").await.unwrap();
    let (_, member) = auth.issue_web_session("member").await.unwrap();
    let (_, outsider) = auth.issue_web_session("outsider").await.unwrap();
    let service = CommunityService::new(
        pool.clone(),
        auth,
        crate::engine::write_admission::WriteAdmission::new(pool.clone()),
    );
    assert!(matches!(
        service
            .create_template(
                &manager,
                &server_id("source"),
                "Must not expose private config",
                None,
            )
            .await,
        Err(CommunityError::Forbidden)
    ));
    let created = service
        .create_template(&owner, &server_id("source"), "Private admin template", None)
        .await
        .unwrap();
    let config_json: String = sqlx::query_scalar("SELECT config FROM server_templates WHERE id=?")
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let config: TemplateConfig = serde_json::from_str(&config_json).unwrap();
    let secret = config
        .channels
        .iter()
        .find(|channel| channel.id == "secret")
        .unwrap();
    assert_eq!(secret.topic, "Hidden topic");
    assert_eq!(secret.aliases, vec!["vault"]);
    assert_eq!(secret.role_overrides.len(), 2);
    assert!(
        secret
            .role_overrides
            .iter()
            .any(|rule| rule.role_id == "private-role")
    );
    assert!(
        secret
            .role_overrides
            .iter()
            .any(|rule| rule.role_id == "manager-role")
    );
    assert!(matches!(
        service
            .instantiate_template(&manager, &created.id, "Leaked")
            .await,
        Err(CommunityError::Forbidden)
    ));
    assert!(matches!(
        service
            .instantiate_template(&member, &created.id, "Leaked")
            .await,
        Err(CommunityError::Forbidden)
    ));
    assert!(matches!(
        service
            .instantiate_template(&outsider, &created.id, "Leaked")
            .await,
        Err(CommunityError::Forbidden)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM servers WHERE name='Leaked'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_log WHERE action_type='server_template_create' \
             AND target_id=? AND actor_id='owner'",
        )
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    service
        .delete_template(&owner, &server_id("source"), &created.id)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_log WHERE action_type='server_template_delete' \
             AND target_id=? AND actor_id='owner'",
        )
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM server_templates WHERE id=?")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
