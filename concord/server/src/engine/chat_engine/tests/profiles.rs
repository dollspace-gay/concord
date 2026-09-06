use super::*;

#[tokio::test]
async fn profile_projection_requires_self_or_shared_server() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('viewer','viewer'),('shared','shared'),('outsider','outsider')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','viewer','owner'),('server','shared','member')")
        .execute(&pool).await.unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("viewer").await.unwrap();
    let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);

    assert!(engine.get_user_profile(&actor, "viewer").await.is_ok());
    assert!(engine.get_user_profile(&actor, "shared").await.is_ok());
    assert!(engine.get_user_profile(&actor, "outsider").await.is_err());
}

#[tokio::test]
async fn atproto_profile_sync_updates_stable_identity_and_live_projection() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('local-user','alice'),('viewer','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','local-user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('server','local-user','owner'),('server','viewer','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) \
         VALUES('at-account','local-user','atproto','did:plc:alice')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let sync_actor = auth.issue_web_session("local-user").await.unwrap().1;
    let viewer_actor = auth.issue_web_session("viewer").await.unwrap().1;
    let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (viewer_session, mut events) = engine
        .connect(
            Some("viewer".into()),
            "viewer".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(viewer_session, viewer_actor)
        .unwrap();
    engine
        .channels
        .get_mut("channel")
        .unwrap()
        .members
        .insert(viewer_session);

    let did = engine
        .verified_atproto_profile_did(&sync_actor)
        .await
        .unwrap();
    assert_eq!(did, "did:plc:alice");
    let input = crate::engine::profile_sync::BlueskyProfileSyncInput {
        did: "did:plc:alice",
        handle: "alice.test",
        display_name: Some("Alice"),
        description: Some("Synced biography"),
        avatar: Some("https://cdn.test/avatar.jpg"),
        banner: Some("https://cdn.test/banner.jpg"),
        followers_count: 4,
        follows_count: 3,
    };
    let updated = engine
        .apply_atproto_profile_sync(&sync_actor, &did, &input)
        .await
        .unwrap();
    assert_eq!(updated.user_id, "local-user");

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        ChatEvent::UserProfile { profile }
            if profile.user_id == "local-user"
                && profile.bio.as_deref() == Some("Synced biography")
    ));
}

#[tokio::test]
async fn presence_projection_uses_durable_identity_hides_invisible_and_fails_closed() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users(id,username,avatar_url) VALUES \
         ('viewer','Viewer',NULL),('会員識別子','Durable Name','durable-avatar')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role,nickname,avatar_url) VALUES \
         ('server','viewer','owner',NULL,NULL), \
         ('server','会員識別子','member','Server Name','server-avatar')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_presence(user_id,status,requested_status,custom_status,status_emoji) \
         VALUES('会員識別子','invisible','invisible','secret','secret-emoji')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, viewer_actor) = auth.issue_web_session("viewer").await.unwrap();
    let engine = ChatEngine::new(pool.clone(), auth, "test-replay-secret", 4000, 100);
    let (viewer_session, _) = engine
        .connect(
            Some("viewer".into()),
            "Viewer Live".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(viewer_session, viewer_actor)
        .unwrap();
    engine
        .connect(
            Some("会員識別子".into()),
            "Transient Live Name".into(),
            Protocol::WebSocket,
            Some("transient-avatar".into()),
        )
        .unwrap();

    let projected = engine
        .get_server_presences(viewer_session, "server")
        .await
        .unwrap();
    let invisible = projected
        .iter()
        .find(|presence| presence.user_id == "会員識別子")
        .unwrap();
    assert_eq!(invisible.nickname, "Server Name");
    assert_eq!(invisible.avatar_url.as_deref(), Some("server-avatar"));
    assert_eq!(invisible.status, "offline");
    assert_eq!(invisible.custom_status, None);
    assert_eq!(invisible.status_emoji, None);

    pool.close().await;
    assert!(matches!(
        engine.get_server_presences(viewer_session, "server").await,
        Err(error) if error.starts_with("DEPENDENCY_UNAVAILABLE:")
    ));
}
