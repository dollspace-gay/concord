use super::*;

#[tokio::test]
async fn retention_floor_and_access_revocation_require_explicit_resync() {
    let (pool, _, actor, conversation, _, replay) = fixture().await;
    let empty_cursor = replay.snapshot(&actor, &[]).await.unwrap().cursor;
    sqlx::query("UPDATE event_retention_state SET retained_from_sequence=2 WHERE singleton=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        replay.replay(&actor, &[], &empty_cursor, 100).await,
        Err(ReplayError::ResyncRequired(ResyncReason::CursorExpired))
    ));

    sqlx::query("UPDATE event_retention_state SET retained_from_sequence=0 WHERE singleton=1")
        .execute(&pool)
        .await
        .unwrap();
    let cursor = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    sqlx::query("DELETE FROM server_members WHERE server_id='server' AND user_id='user'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        replay.replay(&actor, &[conversation], &cursor, 100).await,
        Err(ReplayError::ResyncRequired(ResyncReason::AccessRevoked))
    ));
}
