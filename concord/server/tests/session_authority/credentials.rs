use super::*;

#[tokio::test]
async fn unregistered_legacy_jwt_is_rejected() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool, "session-secret".into(), 1);
    let token = create_session_token("user-1", "session-secret", 1).unwrap();

    assert!(matches!(
        service.authenticate_web_session(&token).await,
        Err(AuthError::Invalid)
    ));
}

#[tokio::test]
async fn revocation_is_durable_and_cancels_live_lease() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = service.issue_web_session("user-1").await.unwrap();
    let lease = service.register_live(&actor).await.unwrap();

    assert!(
        service
            .revoke_credential(actor.credential_id())
            .await
            .unwrap()
    );
    tokio::time::timeout(Duration::from_secs(1), lease.cancelled())
        .await
        .unwrap();
    assert!(matches!(
        AuthService::new(pool, "session-secret".into(), 1)
            .authenticate_web_session(&token)
            .await,
        Err(AuthError::Revoked)
    ));
}

#[tokio::test]
async fn revocation_cancels_every_live_transport_for_exact_credential_only() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool, "session-secret".into(), 1);
    let (_, revoked_actor) = service.issue_web_session("user-1").await.unwrap();
    let (_, retained_actor) = service.issue_web_session("user-1").await.unwrap();
    let first_transport = service.register_live(&revoked_actor).await.unwrap();
    let second_transport = service.register_live(&revoked_actor).await.unwrap();
    let retained_transport = service.register_live(&retained_actor).await.unwrap();

    assert!(
        service
            .revoke_credential(revoked_actor.credential_id())
            .await
            .unwrap()
    );
    tokio::time::timeout(Duration::from_secs(1), first_transport.cancelled())
        .await
        .expect("first transport did not observe credential revocation");
    tokio::time::timeout(Duration::from_secs(1), second_transport.cancelled())
        .await
        .expect("second transport did not observe credential revocation");
    assert!(
        tokio::time::timeout(Duration::from_millis(25), retained_transport.cancelled())
            .await
            .is_err(),
        "revoking one credential cancelled another credential for the same user"
    );
    service.validate_actor(&retained_actor).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_issue_and_revoke_all_has_a_linearizable_result() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool, "session-secret".into(), 1);
    let existing = auth.issue_web_session("user-1").await.unwrap().1;
    let lease = auth.register_live(&existing).await.unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let issuer = {
        let auth = auth.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            auth.issue_web_session("user-1").await.unwrap()
        })
    };
    let revoker = {
        let auth = auth.clone();
        let barrier = barrier.clone();
        let user_id = existing.user_id().clone();
        tokio::spawn(async move {
            barrier.wait().await;
            auth.revoke_all_for_user(&user_id).await.unwrap()
        })
    };
    barrier.wait().await;
    let (token, raced_actor) = issuer.await.unwrap();
    let revoked_count = revoker.await.unwrap();
    assert!(revoked_count >= 1);
    tokio::time::timeout(Duration::from_secs(1), lease.cancelled())
        .await
        .expect("pre-existing live credential was not cancelled");

    let durable_result = auth.authenticate_web_session(&token).await;
    assert!(
        matches!(durable_result, Ok(ref actor) if actor.credential_id() == raced_actor.credential_id())
            || matches!(durable_result, Err(AuthError::Revoked)),
        "concurrent issuance must linearize entirely before or after revoke-all"
    );
}

#[tokio::test]
async fn expiry_and_disabled_account_are_checked_from_durable_state() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let service = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (expired_token, expired_actor) = service.issue_web_session("user-1").await.unwrap();
    sqlx::query(
        "UPDATE auth_credentials SET expires_at=unixepoch()-1, version=version+1 WHERE id=?",
    )
    .bind(expired_actor.credential_id().as_str())
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        service.authenticate_web_session(&expired_token).await,
        Err(AuthError::Expired)
    ));

    let (disabled_token, _) = service.issue_web_session("user-1").await.unwrap();
    sqlx::query("UPDATE users SET disabled_at=datetime('now') WHERE id='user-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        service.authenticate_web_session(&disabled_token).await,
        Err(AuthError::Disabled)
    ));
}

#[tokio::test]
async fn indexed_irc_and_bot_tokens_use_shared_authority() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    sqlx::query("UPDATE users SET is_bot=1 WHERE id='user-1'")
        .execute(&pool)
        .await
        .unwrap();
    let service = AuthService::new(pool, "session-secret".into(), 1);
    let web = service.issue_web_session("user-1").await.unwrap().1;

    let irc = service
        .issue_irc_token(web.user_id(), Some("terminal"))
        .await
        .unwrap();
    let irc_actor = service
        .authenticate_irc(&irc.secret, "carmilla")
        .await
        .unwrap();
    assert_eq!(irc_actor.kind(), CredentialKind::IrcToken);

    let bot = service
        .issue_bot_token(web.user_id(), "automation", "messages.read messages.write")
        .await
        .unwrap();
    let bot_actor = service.authenticate_bot(&bot.secret).await.unwrap();
    assert!(bot_actor.scopes().contains("messages.read"));
}

#[tokio::test]
async fn bot_credentials_are_gated_by_stable_owner_not_server_management() {
    let pool = database().await;
    user(&pool, "owner", "owner-nick").await;
    user(&pool, "other", "other-nick").await;
    bots::create_bot_user_owned(&pool, "bot-1", "owned-bot", None, "owner")
        .await
        .unwrap();
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let engine = ChatEngine::new(pool.clone(), auth, "session-secret", 4000, 100);
    let owner_session = engine
        .connect(
            Some("owner".into()),
            "owner-nick".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap()
        .0;
    let other_session = engine
        .connect(
            Some("other".into()),
            "other-nick".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap()
        .0;

    assert!(
        engine
            .create_bot_token(other_session, "bot-1", "forbidden", None)
            .await
            .unwrap_err()
            .contains("recorded bot owner")
    );
    engine
        .create_bot_token(owner_session, "bot-1", "owner-token", Some("messages.read"))
        .await
        .unwrap();
    assert_eq!(
        bots::list_bot_tokens(&pool, "bot-1").await.unwrap().len(),
        1
    );
}
