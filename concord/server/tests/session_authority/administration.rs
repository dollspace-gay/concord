use super::*;

#[tokio::test]
async fn incoming_webhook_route_is_scoped_idempotent_and_revocable() {
    let pool = database().await;
    user(&pool, "owner", "owner").await;
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
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('allowed','server','#allowed'),('wrong','server','#wrong')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind((concord_server::engine::permissions::Permissions::VIEW_CHANNELS
            | concord_server::engine::permissions::Permissions::SEND_MESSAGES
            | concord_server::engine::permissions::Permissions::READ_MESSAGE_HISTORY).bits() as i64)
        .execute(&pool).await.unwrap();

    let principal = "webhook:fixture";
    bots::create_bot_user_owned(&pool, principal, "fixture-hook", None, "owner")
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server',?,'member')")
        .bind(principal)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) VALUES('install',?,'server','owner','webhook:channel:allowed','active')")
        .bind(principal).execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let principal_id = concord_server::auth::authority::UserId::from_stored(principal).unwrap();
    let issued = auth
        .issue_bot_token(&principal_id, "Webhook", "bot webhook:channel:allowed")
        .await
        .unwrap();
    let webhook_id = Uuid::new_v4().to_string();
    concord_server::db::queries::webhooks::create_webhook(
        &pool,
        &concord_server::db::models::CreateWebhookParams {
            id: &webhook_id,
            server_id: "server",
            channel_id: "allowed",
            name: "Hook",
            avatar_url: None,
            webhook_type: "incoming",
            token: issued.credential_id.as_str(),
            url: None,
            created_by: "owner",
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE webhooks SET credential_id=?,principal_user_id=?,credential_state='active' WHERE id=?")
        .bind(issued.credential_id.as_str()).bind(principal).bind(&webhook_id)
        .execute(&pool).await.unwrap();
    let router = app(pool.clone(), auth.clone()).await;

    let (status, first) =
        execute_webhook(&router, &webhook_id, &issued.secret, "stable-key", "hello").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(first["replayed"], false);
    let (status, replay) =
        execute_webhook(&router, &webhook_id, &issued.secret, "stable-key", "hello").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(replay["replayed"], true);
    assert_eq!(first["message_id"], replay["message_id"]);

    user(&pool, "new-owner", "new-owner").await;
    sqlx::query("UPDATE servers SET owner_id='new-owner' WHERE id='server'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = execute_webhook(
        &router,
        &webhook_id,
        &issued.secret,
        "after-owner-loss",
        "still active",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = execute_webhook(
        &router,
        &webhook_id,
        &issued.secret,
        "stable-key",
        "changed",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE sender_id=?")
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);

    sqlx::query("INSERT INTO automod_rules(id,server_id,name,rule_type,config) VALUES('blocked-word','server','blocked','keyword','{\"words\":[\"forbidden\"]}')")
        .execute(&pool).await.unwrap();
    let (status, _) =
        execute_webhook(&router, &webhook_id, &issued.secret, "automod", "forbidden").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    sqlx::query("UPDATE channels SET slowmode_seconds=60 WHERE id='allowed'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) =
        execute_webhook(&router, &webhook_id, &issued.secret, "slowmode", "too soon").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    sqlx::query("UPDATE webhooks SET channel_id='wrong' WHERE id=?")
        .bind(&webhook_id)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) =
        execute_webhook(&router, &webhook_id, &issued.secret, "wrong-channel", "no").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    sqlx::query("UPDATE webhooks SET channel_id='allowed' WHERE id=?")
        .bind(&webhook_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now') WHERE id='install'")
        .execute(&pool).await.unwrap();
    let (status, _) = execute_webhook(&router, &webhook_id, &issued.secret, "revoked", "no").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authenticated_route_reports_session_database_outage_as_unavailable() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let router = app(pool.clone(), auth).await;
    pool.close().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn web_session_is_recorded_and_survives_service_restart() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let first = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, issued) = first.issue_web_session("user-1").await.unwrap();

    let restarted = AuthService::new(pool, "session-secret".into(), 1);
    let restored = restarted.authenticate_web_session(&token).await.unwrap();
    assert_eq!(restored.user_id(), issued.user_id());
    assert_eq!(restored.credential_id(), issued.credential_id());
}

#[tokio::test]
async fn legacy_bot_hint_and_changed_irc_handle_remain_compatible() {
    let pool = database().await;
    user(&pool, "target", "old-handle").await;
    sqlx::query("UPDATE users SET is_bot=1 WHERE id='target'")
        .execute(&pool)
        .await
        .unwrap();
    let bot_secret = "bot_target.legacy-secret";
    let bot_hash = hash_irc_token(bot_secret).unwrap();
    sqlx::query(
        "INSERT INTO auth_credentials(id,user_id,kind,secret_hash,scopes,legacy_source_id) \
         VALUES ('legacy-bot-credential','target','bot_token',?,'messages.read','legacy-bot')",
    )
    .bind(&bot_hash)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO bot_tokens(id,user_id,token_hash,name,scopes,credential_id) \
         VALUES ('legacy-bot','target',?,'legacy','messages.read','legacy-bot-credential')",
    )
    .bind(&bot_hash)
    .execute(&pool)
    .await
    .unwrap();

    let irc_secret = "legacy-irc-secret";
    let irc_hash = hash_irc_token(irc_secret).unwrap();
    sqlx::query(
        "INSERT INTO auth_credentials(id,user_id,kind,secret_hash,scopes,legacy_source_id) \
         VALUES ('legacy-irc-credential','target','irc_token',?,'irc','legacy-irc')",
    )
    .bind(&irc_hash)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO irc_tokens(id,user_id,token_hash,label,credential_id) \
         VALUES ('legacy-irc','target',?,'legacy','legacy-irc-credential')",
    )
    .bind(&irc_hash)
    .execute(&pool)
    .await
    .unwrap();
    users::update_username(&pool, "target", "new-handle")
        .await
        .unwrap();

    let service = AuthService::new(pool, "session-secret".into(), 1);
    assert_eq!(
        service
            .authenticate_bot(bot_secret)
            .await
            .unwrap()
            .user_id()
            .as_str(),
        "target"
    );
    assert_eq!(
        service
            .authenticate_irc(irc_secret, "new-handle")
            .await
            .unwrap()
            .user_id()
            .as_str(),
        "target"
    );
}

#[tokio::test]
async fn pins_require_manage_messages_and_bookmarks_redact_then_allow_private_cleanup() {
    let pool = database().await;
    user(&pool, "owner", "owner").await;
    user(&pool, "member", "member").await;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind(DEFAULT_EVERYONE.bits() as i64).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','member','member','private content')")
        .execute(&pool).await.unwrap();

    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let actor = auth.issue_web_session("member").await.unwrap().1;
    let engine = ChatEngine::new(pool.clone(), auth.clone(), "session-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (session_id, mut events) = engine
        .connect(
            Some("member".into()),
            "member".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine.bind_authenticated_actor(session_id, actor).unwrap();

    assert!(
        engine
            .pin_message(session_id, "server", "#general", "message")
            .await
            .is_err()
    );
    let pin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pinned_messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pin_count, 0, "message owner bypassed MANAGE_MESSAGES");

    engine
        .add_bookmark(session_id, "message", Some("remember"))
        .await
        .unwrap();
    assert!(matches!(
        events.recv().await,
        Some(ChatEvent::BookmarkAdd { .. })
    ));
    sqlx::query("UPDATE messages SET deleted_at=datetime('now') WHERE id='message'")
        .execute(&pool)
        .await
        .unwrap();
    engine.list_bookmarks(session_id).await.unwrap();
    let Some(ChatEvent::BookmarkList { bookmarks }) = events.recv().await else {
        panic!("bookmark list was not delivered");
    };
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].content, "[deleted]");

    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('deny-history','channel','role','everyone',?)")
        .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64).execute(&pool).await.unwrap();
    engine.remove_bookmark(session_id, "message").await.unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE user_id='member'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        remaining, 0,
        "history revocation prevented private bookmark cleanup"
    );
}
