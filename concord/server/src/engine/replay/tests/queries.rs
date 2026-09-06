use super::*;

#[tokio::test]
async fn snapshot_byte_budget_supports_smaller_page_and_marks_older_history() {
    let (pool, _, actor, first_conversation, _, replay) = fixture().await;
    let mut conversations = vec![first_conversation];
    for channel_index in 1..10 {
        let channel_id = format!("channel-{channel_index}");
        let channel_name = format!("#channel-{channel_index}");
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES(?,'server',?)")
            .bind(&channel_id)
            .bind(channel_name)
            .execute(&pool)
            .await
            .unwrap();
        conversations.push(
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id=?")
                .bind(channel_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
        );
    }
    let content = "🦇".repeat(4000);
    for (conversation_index, conversation_id) in conversations.iter().enumerate() {
        let channel_id = if conversation_index == 0 {
            "channel".to_owned()
        } else {
            format!("channel-{conversation_index}")
        };
        for sequence in 1..=10 {
            sqlx::query(
                "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
                                      conversation_id,conversation_sequence,content_format) \
                 VALUES(?,'server',?,'user','carmilla',?,?,?,'plain')",
            )
            .bind(format!("large-{conversation_index}-{sequence}"))
            .bind(&channel_id)
            .bind(&content)
            .bind(conversation_id)
            .bind(sequence)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    let snapshot = replay
        .snapshot_with_limit(&actor, &conversations, 100)
        .await
        .unwrap();
    assert!(snapshot.messages.len() < 100);
    assert!(!snapshot.messages.is_empty());
    assert_eq!(snapshot.history_before.len(), 10);
    let wire_bytes = serde_json::to_vec(&crate::engine::events::ChatEvent::SyncSnapshot {
        request_id: "request".into(),
        snapshot,
    })
    .unwrap()
    .len();
    assert!(wire_bytes < crate::engine::user_session::MAX_OUTBOUND_BYTES);
}
