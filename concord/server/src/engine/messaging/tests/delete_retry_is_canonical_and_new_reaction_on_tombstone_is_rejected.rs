use super::*;

#[tokio::test]
async fn delete_retry_is_canonical_and_new_reaction_on_tombstone_is_rejected() {
    let (pool, _, actor, service) = fixture().await;
    let sent = service
        .send_channel_message(&actor, command("send", "send-client", "hello"))
        .await
        .unwrap();
    let delete = EntityCommand {
        request_id: "delete-1",
        client_message_id: "delete-client",
        operation_generation: None,
        message_id: &sent.message_id,
    };
    let original = service
        .delete_message(&actor, delete)
        .await
        .unwrap()
        .receipt;
    let retry = service
        .delete_message(
            &actor,
            EntityCommand {
                request_id: "delete-2",
                client_message_id: "delete-client",
                operation_generation: None,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap()
        .receipt;
    assert!(retry.replayed);
    assert_eq!(retry.request_id, "delete-2");
    assert_eq!(
        retry.event_sequence_internal,
        original.event_sequence_internal
    );

    let reaction = service
        .change_reaction(
            &actor,
            ReactionCommand {
                request_id: "reaction",
                client_message_id: "reaction-client",
                operation_generation: None,
                message_id: &sent.message_id,
                emoji: "heart",
            },
            true,
        )
        .await;
    assert!(matches!(reaction, Err(MessagingError::Unavailable)));
    let reaction_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reaction_count, 0);
}

#[tokio::test]
async fn mutation_event_failure_rolls_back_projection_and_receipt() {
    let (pool, _, actor, service) = fixture().await;
    let sent = service
        .send_channel_message(&actor, command("send", "send-client", "before"))
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_edit_event BEFORE INSERT ON event_log \
         WHEN NEW.event_kind='message_edited' \
         BEGIN SELECT RAISE(ABORT,'forced edit event failure'); END",
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
                content: "after",
                content_format: ContentFormat::Plain,
                mentions: &[],
            },
        )
        .await;
    assert!(matches!(result, Err(MessagingError::Internal(_))));
    let state: (String, i64, i64) = sqlx::query_as(
        "SELECT content,entity_version, \
            (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='edit') \
         FROM messages WHERE id=?",
    )
    .bind(&sent.message_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("before".into(), 1, 0));
}
