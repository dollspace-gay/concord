use super::*;

#[tokio::test]
async fn automod_timeout_reject_commits_timeout_and_deduplicated_audit_only() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query(
        "INSERT INTO automod_rules( \
            id,server_id,name,rule_type,config,action_type,timeout_duration_seconds \
         ) VALUES('rule','server','No spam','keyword', \
                  '{\"words\":[\"spam\"]}','timeout',60)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let result = service
        .send_channel_message(&actor, command("send-one", "same-client", "spam"))
        .await;
    assert!(matches!(result, Err(MessagingError::AutoModRejected(_))));
    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM messages WHERE content='spam'), \
            (SELECT count(*) FROM command_receipts WHERE client_message_id='same-client'), \
            (SELECT count(*) FROM audit_log WHERE action_type='automod_reject'), \
            (SELECT count(*) FROM server_members \
             WHERE server_id='server' AND user_id='user' AND timeout_until>datetime('now'))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 0, 1, 1));
}
