use super::*;

#[tokio::test]
async fn announcement_publish_is_deduplicated_and_lineage_propagates_after_unfollow() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query("UPDATE channels SET is_announcement=1 WHERE id='channel'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('target-server','Target','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('target-server','user','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channels(id,server_id,name) VALUES('target-channel','target-server','#news')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channel_follows(id,source_channel_id,target_channel_id,created_by) VALUES('follow','channel','target-channel','user')")
        .execute(&pool)
        .await
        .unwrap();
    let source = service
        .send_channel_message(&actor, command("send-request", "send-client", "original"))
        .await
        .unwrap();

    let first = service
        .publish_announcement(
            &actor,
            PublishAnnouncementCommand {
                message_id: &source.message_id,
            },
        )
        .await
        .unwrap();
    let retry = service
        .publish_announcement(
            &actor,
            PublishAnnouncementCommand {
                message_id: &source.message_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(first.len(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM announcement_publications")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );

    sqlx::query("DELETE FROM channel_follows WHERE id='follow'")
        .execute(&pool)
        .await
        .unwrap();
    service
        .edit_message(
            &actor,
            EditMessageCommand {
                request_id: "edit-request",
                client_message_id: "edit-client",
                operation_generation: None,
                message_id: &source.message_id,
                content: "corrected",
                content_format: ContentFormat::Markdown,
                mentions: &[],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id=?")
            .bind(&first[0].target_message_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "corrected"
    );
    service
        .delete_message(
            &actor,
            EntityCommand {
                request_id: "delete-request",
                client_message_id: "delete-client",
                operation_generation: None,
                message_id: &source.message_id,
            },
        )
        .await
        .unwrap();
    let state: (String, i64, i64) = sqlx::query_as(
        "SELECT ap.state,ap.source_version,(m.deleted_at IS NOT NULL) \
         FROM announcement_publications ap JOIN messages m ON m.id=ap.target_message_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("deleted".into(), 3, 1));
}

#[tokio::test]
async fn pool_exhaustion_is_bounded_by_the_overall_admission_deadline() {
    let (pool, _, actor, service) = fixture().await;
    let mut held = Vec::new();
    for _ in 0..5 {
        held.push(pool.acquire().await.unwrap());
    }
    let started = std::time::Instant::now();
    let result = service
        .send_channel_message(&actor, command("request", "client", "hello"))
        .await;
    assert!(matches!(result, Err(MessagingError::DependencyUnavailable)));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    drop(held);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn entity_tuple_ids_are_unambiguous_with_colon_bearing_fields() {
    assert_ne!(
        reaction_entity_id("message:part", "did:plc:user", "emoji"),
        reaction_entity_id("message", "part:did:plc:user", "emoji")
    );
    assert_ne!(
        read_entity_id("did:plc:user", "direct:a:b"),
        read_entity_id("did", "plc:user:direct:a:b")
    );
    assert_ne!(
        reaction_entity_id("same", "tuple", "value"),
        read_entity_id("same", "tuple:value")
    );
}

#[tokio::test]
async fn already_read_legacy_state_succeeds_without_creating_durable_churn() {
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
    sqlx::query(
        "INSERT INTO read_states( \
             user_id,channel_id,last_read_message_id,conversation_sequence \
         ) VALUES('user','channel',?,1)",
    )
    .bind(&sent.message_id)
    .execute(&pool)
    .await
    .unwrap();

    service
        .mark_read(
            &actor,
            ReadCommand {
                request_id: "legacy-read",
                client_message_id: "legacy-read-client",
                operation_generation: None,
                conversation_id: &conversation_id,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap();
    let state: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT COUNT(*) FROM event_log WHERE entity_type='read_state'), \
            (SELECT COUNT(*) FROM entity_versions WHERE entity_type='read_state'), \
            (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='read')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 0, 1));
}
