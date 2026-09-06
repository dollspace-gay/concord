use super::*;

#[tokio::test]
async fn message_event_atomically_enqueues_subscribed_outgoing_webhook() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query(
        "INSERT INTO channels(id,server_id,name,is_private) VALUES \
         ('sibling','server','#sibling',0),('private','server','#private',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhooks( \
            id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
         ) VALUES \
            ('hook','server','channel','Hook','outgoing','hash-main','https://example.com/hook','user','active'), \
            ('sibling-hook','server','sibling','Sibling','outgoing','hash-sibling','https://example.com/sibling','user','active'), \
            ('private-hook','server','private','Private','outgoing','hash-private','https://example.com/private','user','active')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhook_events(id,webhook_id,event_type) VALUES \
         ('subscription','hook','message_create'), \
         ('sibling-subscription','sibling-hook','message_create'), \
         ('private-subscription','private-hook','message_create')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let receipt = service
        .send_channel_message(&actor, command("request", "client", "hello"))
        .await
        .unwrap();
    let delivery: (String, String, String, i64) = sqlx::query_as(
        "SELECT d.state,j.state,j.destination_grant,d.event_sequence \
         FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(delivery.0, "pending");
    assert_eq!(delivery.1, "pending");
    assert_eq!(delivery.2, "webhook:hook:1");
    assert_eq!(delivery.3.to_string(), receipt.sequence);
    let payload: String = sqlx::query_scalar("SELECT payload_json FROM webhook_deliveries")
        .fetch_one(&pool)
        .await
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["channel_id"], "channel");
    assert!(payload.get("event_sequence").is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM webhook_deliveries")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn send_commits_message_receipt_event_and_outbox_before_returning() {
    let (pool, _, actor, service) = fixture().await;
    let receipt = service
        .send_channel_message(&actor, command("request", "client", "hello"))
        .await
        .unwrap();
    assert_eq!(receipt.sequence, "1");
    assert_eq!(receipt.event_sequence_internal, 1);
    assert!(!receipt.replayed);
    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT COUNT(*) FROM messages WHERE id=?), \
            (SELECT COUNT(*) FROM command_receipts WHERE canonical_message_id=?), \
            (SELECT COUNT(*) FROM event_log WHERE entity_id=?), \
            (SELECT COUNT(*) FROM delivery_outbox WHERE event_sequence=?)",
    )
    .bind(&receipt.message_id)
    .bind(&receipt.message_id)
    .bind(&receipt.message_id)
    .bind(receipt.event_sequence_internal as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (1, 1, 1, 1));
}

#[tokio::test]
async fn retained_receipt_wins_across_database_restore_and_operation_rollover() {
    let (pool, _, actor, service) = fixture().await;
    let original_generation: String = sqlx::query_scalar(
        "SELECT current_generation FROM operation_generation_state WHERE singleton=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let original = service
        .send_channel_message(
            &actor,
            command_in_generation("request-1", "stable-client", "hello", &original_generation),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE database_metadata SET generation='restored-database' WHERE singleton=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO operation_generations(generation,issued_at,expires_at) \
         VALUES('next-operation-generation',unixepoch(),unixepoch()+604800)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE operation_generation_state SET current_generation='next-operation-generation' \
         WHERE singleton=1",
    )
    .execute(&pool)
    .await
    .unwrap();

    let retry = service
        .send_channel_message(
            &actor,
            command_in_generation("request-2", "stable-client", "hello", &original_generation),
        )
        .await
        .unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.message_id, original.message_id);
    assert!(matches!(
        service
            .send_channel_message(
                &actor,
                command_in_generation(
                    "request-3",
                    "stable-client",
                    "different",
                    "next-operation-generation",
                ),
            )
            .await,
        Err(MessagingError::IdempotencyConflict)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_sends_allocate_unique_monotonic_sequences() {
    let (_, _, actor, service) = fixture().await;
    let first = {
        let actor = actor.clone();
        let service = service.clone();
        tokio::spawn(async move {
            service
                .send_channel_message(&actor, command("r1", "c1", "one"))
                .await
                .unwrap()
        })
    };
    let second = {
        let actor = actor.clone();
        let service = service.clone();
        tokio::spawn(async move {
            service
                .send_channel_message(&actor, command("r2", "c2", "two"))
                .await
                .unwrap()
        })
    };
    let mut sequences = [
        first.await.unwrap().sequence.parse::<u64>().unwrap(),
        second.await.unwrap().sequence.parse::<u64>().unwrap(),
    ];
    sequences.sort_unstable();
    assert_eq!(sequences, [1, 2]);
}

#[tokio::test]
async fn direct_send_resolves_alias_and_commits_one_canonical_conversation() {
    let (pool, _, actor, service) = fixture().await;
    let receipt = service
        .send_direct_message(
            &actor,
            SendDirectMessageCommand {
                request_id: "direct-1",
                client_message_id: "direct-client",
                operation_generation: None,
                recipient: "LaUrElAi",
                content: "hello",
                content_format: ContentFormat::Plain,
                reply_to_id: None,
                attachment_ids: &[],
            },
        )
        .await
        .unwrap();
    let row: (String, String, String, i64) = sqlx::query_as(
        "SELECT m.conversation_id,m.target_user_id,m.content, \
                (SELECT COUNT(*) FROM conversation_participants cp \
                 WHERE cp.conversation_id=m.conversation_id) \
         FROM messages m WHERE m.id=?",
    )
    .bind(&receipt.message_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.0.starts_with("direct:"));
    assert_eq!(row.1, "other");
    assert_eq!(row.2, "hello");
    assert_eq!(row.3, 2);

    let retry = service
        .send_direct_message(
            &actor,
            SendDirectMessageCommand {
                request_id: "direct-2",
                client_message_id: "direct-client",
                operation_generation: None,
                recipient: "other",
                content: "hello",
                content_format: ContentFormat::Plain,
                reply_to_id: None,
                attachment_ids: &[],
            },
        )
        .await
        .unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.message_id, receipt.message_id);
}

#[tokio::test]
async fn direct_send_reuses_existing_opaque_pair_without_creating_an_orphan() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query("INSERT INTO conversations(id,kind) VALUES('opaque-direct','direct')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO direct_conversation_pairs( \
             conversation_id,lower_user_id,upper_user_id \
         ) VALUES('opaque-direct','other','user')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversation_participants(conversation_id,user_id) \
         VALUES('opaque-direct','other'),('opaque-direct','user')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let receipt = service
        .send_direct_message(
            &actor,
            SendDirectMessageCommand {
                request_id: "opaque-direct-request",
                client_message_id: "opaque-direct-client",
                operation_generation: None,
                recipient: "other",
                content: "existing history",
                content_format: ContentFormat::Plain,
                reply_to_id: None,
                attachment_ids: &[],
            },
        )
        .await
        .unwrap();

    let message_conversation: String =
        sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
            .bind(&receipt.message_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let direct_conversations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE kind='direct'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(message_conversation, "opaque-direct");
    assert_eq!(direct_conversations, 1);
}

#[tokio::test]
async fn deleting_a_channel_message_does_not_reset_slow_mode() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query("UPDATE channels SET slowmode_seconds=60 WHERE id='channel'")
        .execute(&pool)
        .await
        .unwrap();
    let sent = service
        .send_channel_message(&actor, command("slow-send", "slow-client", "one"))
        .await
        .unwrap();
    service
        .delete_message(
            &actor,
            EntityCommand {
                request_id: "slow-delete",
                client_message_id: "slow-delete-client",
                operation_generation: None,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        service
            .send_channel_message(&actor, command("slow-send-2", "slow-client-2", "two"))
            .await,
        Err(MessagingError::RateLimited)
    ));
}

#[tokio::test]
async fn deleted_channel_messages_still_consume_the_send_rate_budget() {
    let (_, _, actor, service) = fixture().await;
    for index in 0..RATE_WINDOW_MESSAGES {
        let request = format!("rate-send-{index}");
        let client = format!("rate-client-{index}");
        let sent = service
            .send_channel_message(&actor, command(&request, &client, "message"))
            .await
            .unwrap();
        let delete_request = format!("rate-delete-{index}");
        let delete_client = format!("rate-delete-client-{index}");
        service
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: &delete_request,
                    client_message_id: &delete_client,
                    operation_generation: None,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        service
            .send_channel_message(&actor, command("rate-over", "rate-over-client", "blocked"))
            .await,
        Err(MessagingError::RateLimited)
    ));
}

#[tokio::test]
async fn deleted_direct_messages_still_consume_the_send_rate_budget() {
    let (_, _, actor, service) = fixture().await;
    for index in 0..RATE_WINDOW_MESSAGES {
        let request = format!("dm-send-{index}");
        let client = format!("dm-client-{index}");
        let sent = service
            .send_direct_message(
                &actor,
                SendDirectMessageCommand {
                    request_id: &request,
                    client_message_id: &client,
                    operation_generation: None,
                    recipient: "other",
                    content: "message",
                    content_format: ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
            )
            .await
            .unwrap();
        let delete_request = format!("dm-delete-{index}");
        let delete_client = format!("dm-delete-client-{index}");
        service
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: &delete_request,
                    client_message_id: &delete_client,
                    operation_generation: None,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        service
            .send_direct_message(
                &actor,
                SendDirectMessageCommand {
                    request_id: "dm-over",
                    client_message_id: "dm-over-client",
                    operation_generation: None,
                    recipient: "other",
                    content: "blocked",
                    content_format: ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
            )
            .await,
        Err(MessagingError::RateLimited)
    ));
}
