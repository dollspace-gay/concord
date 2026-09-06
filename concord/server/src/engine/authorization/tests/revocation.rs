use super::*;

#[tokio::test]
async fn bot_authority_intersects_credential_installation_and_exact_webhook_scope() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot','bot',1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','bot','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = AuthService::new(pool.clone(), "bot-secret".into(), 1);
    let bot_id = crate::auth::authority::UserId::from_stored("bot").unwrap();

    sqlx::query("INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) VALUES('install','bot','server','owner','messages','active')")
        .execute(&pool).await.unwrap();
    let transport_only = auth
        .issue_bot_token(&bot_id, "transport", "bot")
        .await
        .unwrap();
    let actor = auth.authenticate_bot(&transport_only.secret).await.unwrap();
    assert!(
        service
            .authorize_actor(&auth, &actor, "public", ChannelAction::Send)
            .await
            .is_err()
    );

    let usable = auth
        .issue_bot_token(&bot_id, "usable", "bot messages")
        .await
        .unwrap();
    let actor = auth.authenticate_bot(&usable.secret).await.unwrap();
    service
        .authorize_actor(&auth, &actor, "public", ChannelAction::Send)
        .await
        .unwrap();
    sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now'),authorization_version=authorization_version+1 WHERE id='install'")
        .execute(&pool).await.unwrap();
    assert!(
        service
            .authorize_actor(&auth, &actor, "public", ChannelAction::Send)
            .await
            .is_err()
    );

    sqlx::query("UPDATE bot_installations SET state='active',revoked_at=NULL,granted_scopes='webhook:channel:public',authorization_version=authorization_version+1 WHERE id='install'")
        .execute(&pool).await.unwrap();
    let exact = auth
        .issue_bot_token(&bot_id, "webhook", "bot webhook:channel:public")
        .await
        .unwrap();
    let exact = auth.authenticate_bot(&exact.secret).await.unwrap();
    service
        .authorize_actor(&auth, &exact, "public", ChannelAction::Send)
        .await
        .unwrap();
    assert!(
        service
            .authorize_actor(&auth, &exact, "public", ChannelAction::View)
            .await
            .is_err()
    );
}
