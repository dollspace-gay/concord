use super::*;

#[tokio::test]
async fn read_state_replay_is_private_to_owning_principal() {
    let (_, auth, actor, conversation, messaging, replay) = fixture().await;
    let sent = send(&messaging, &actor, "send", "message").await;
    let other = auth.issue_web_session("other").await.unwrap().1;
    let other_cursor = replay
        .snapshot(&other, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    messaging
        .mark_read(
            &actor,
            ReadCommand {
                request_id: "read",
                client_message_id: "read",
                operation_generation: None,
                conversation_id: &conversation,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap();
    let batch = replay
        .replay(&other, &[conversation], &other_cursor, 100)
        .await
        .unwrap();
    assert!(batch.events.is_empty());
    assert!(!batch.has_more);
}

#[tokio::test]
async fn snapshot_reactions_are_scoped_to_the_bounded_message_window() {
    let (pool, _, actor, conversation, _, replay) = fixture().await;
    sqlx::query(
        "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<101) \
         INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at, \
                              conversation_id,conversation_sequence,content_format,entity_version) \
         SELECT printf('history-%03d',n),'server','channel','user','carmilla','history', \
                datetime('now'),'channel:' || hex(CAST('channel' AS BLOB)),n,'plain',1 FROM numbers",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reactions(message_id,user_id,emoji) VALUES \
         ('history-001','user','old'),('history-101','user','bat'), \
         ('history-101','other','bat')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let snapshot = replay.snapshot(&actor, &[conversation]).await.unwrap();
    assert_eq!(snapshot.messages.len(), 100);
    assert_eq!(
        snapshot.reactions,
        vec![SnapshotReactionGroup {
            message_id: "history-101".into(),
            emoji: "bat".into(),
            count: 2,
            reacted_by_me: true,
        }]
    );
    assert!(
        snapshot
            .messages
            .iter()
            .all(|message| message.message_id != "history-001")
    );
}
