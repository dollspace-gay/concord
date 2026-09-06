use super::*;

#[tokio::test]
async fn direct_conversation_send_enforces_participants_blocks_and_preferences() {
    let (pool, service) = fixture().await;
    sqlx::query("INSERT INTO conversations(id,kind) VALUES('dm','direct')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO conversation_participants(conversation_id,user_id) VALUES('dm','owner'),('dm','member')")
        .execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("member").await.unwrap();

    let authorize = async || {
        let mut transaction = pool.begin().await.unwrap();
        service
            .authorize_conversation_actor_in(
                &mut transaction,
                &auth,
                &actor,
                "dm",
                ConversationAction::Send,
            )
            .await
    };
    authorize().await.unwrap();
    sqlx::query(
        "INSERT INTO user_blocks(blocker_user_id,blocked_user_id) VALUES('owner','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        authorize().await,
        Err(AuthorizationError::Unavailable)
    ));
    sqlx::query("DELETE FROM user_blocks")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO direct_message_preferences(user_id,allow_from) VALUES('owner','none')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        authorize().await,
        Err(AuthorizationError::Unavailable)
    ));
    sqlx::query(
        "UPDATE direct_message_preferences SET allow_from='everyone' WHERE user_id='owner'",
    )
    .execute(&pool)
    .await
    .unwrap();
    authorize().await.unwrap();
}
