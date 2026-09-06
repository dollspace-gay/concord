use super::*;

#[tokio::test]
async fn read_state_never_moves_backwards() {
    let (pool, _, actor, service) = fixture().await;
    let first = service
        .send_channel_message(&actor, command("send-1", "send-client-1", "one"))
        .await
        .unwrap();
    let second = service
        .send_channel_message(&actor, command("send-2", "send-client-2", "two"))
        .await
        .unwrap();
    let conversation_id: String =
        sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
            .bind(&first.message_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    for (request_id, client_message_id, message_id) in [
        ("read-2", "read-client-2", second.message_id.as_str()),
        ("read-1", "read-client-1", first.message_id.as_str()),
        (
            "read-2-again",
            "read-client-2-again",
            second.message_id.as_str(),
        ),
    ] {
        service
            .mark_read(
                &actor,
                ReadCommand {
                    request_id,
                    client_message_id,
                    operation_generation: None,
                    conversation_id: &conversation_id,
                    message_id,
                },
            )
            .await
            .unwrap();
    }
    let state: (String, i64) = sqlx::query_as(
        "SELECT last_read_message_id,conversation_sequence FROM read_states \
         WHERE user_id='user' AND channel_id='channel'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (second.message_id, 2));
    let durable_state: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT COUNT(*) FROM event_log WHERE event_kind='read_advanced'), \
            (SELECT version FROM entity_versions WHERE entity_type='read_state'), \
            (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='read')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(durable_state, (1, 1, 3));
}

#[tokio::test]
async fn already_read_state_succeeds_after_transport_event_pruning() {
    let (pool, _, actor, service) = fixture().await;
    let sent = service
        .send_channel_message(&actor, command("send", "send-client", "one"))
        .await
        .unwrap();
    let conversation_id: String =
        sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
            .bind(&sent.message_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    service
        .mark_read(
            &actor,
            ReadCommand {
                request_id: "read",
                client_message_id: "read-client",
                operation_generation: None,
                conversation_id: &conversation_id,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap();
    sqlx::query(
        "DELETE FROM delivery_outbox WHERE event_sequence IN ( \
             SELECT event_sequence FROM event_log WHERE entity_type='read_state' \
         )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM event_log WHERE entity_type='read_state'")
        .execute(&pool)
        .await
        .unwrap();

    service
        .mark_read(
            &actor,
            ReadCommand {
                request_id: "read-again",
                client_message_id: "read-client-again",
                operation_generation: None,
                conversation_id: &conversation_id,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap();
    let state: (i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT COUNT(*) FROM event_log WHERE entity_type='read_state'), \
            (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='read')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 2));
}

#[tokio::test]
async fn automod_rejected_edit_preserves_message_and_has_no_receipt_or_event() {
    let (pool, _, actor, service) = fixture().await;
    let sent = service
        .send_channel_message(&actor, command("send", "send-client", "allowed"))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO automod_rules(id,server_id,name,rule_type,config) \
         VALUES('rule','server','blocked','keyword','{\"words\":[\"forbidden\"]}')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let result = service
        .edit_message(
            &actor,
            EditMessageCommand {
                request_id: "edit",
                client_message_id: "edit-client",
                operation_generation: None,
                message_id: &sent.message_id,
                content: "forbidden",
                content_format: ContentFormat::Plain,
                mentions: &[],
            },
        )
        .await;
    assert!(matches!(result, Err(MessagingError::AutoModRejected(_))));
    let state: (String, i64, i64, i64) = sqlx::query_as(
        "SELECT content, \
            (SELECT COUNT(*) FROM event_log WHERE event_kind='message_edited'), \
            (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='edit'), \
            (SELECT COUNT(*) FROM audit_log WHERE action_type='automod_reject') \
         FROM messages WHERE id=?",
    )
    .bind(&sent.message_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("allowed".into(), 0, 0, 1));
}

#[tokio::test]
async fn automod_flag_accepts_message_without_storing_content_in_audit() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query(
        "INSERT INTO automod_rules( \
            id,server_id,name,rule_type,config,action_type \
         ) VALUES('rule','server','Review links','link_filter', \
                  '{\"block_all\":true}','flag')",
    )
    .execute(&pool)
    .await
    .unwrap();
    service
        .send_channel_message(
            &actor,
            command("flag-send", "flag-client", "https://private.example/path"),
        )
        .await
        .unwrap();
    let details: String =
        sqlx::query_scalar("SELECT changes FROM audit_log WHERE action_type='automod_flag'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(details.contains("Review links"));
    assert!(!details.contains("private.example"));
}
