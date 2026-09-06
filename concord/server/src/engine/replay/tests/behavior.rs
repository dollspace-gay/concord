use super::*;

#[tokio::test]
async fn replay_preserves_supported_historical_conversation_ids() {
    let (pool, _, actor, conversation, messaging, replay) = fixture().await;
    let historical = format!(" historical conversation:{} ", "界".repeat(300));
    sqlx::query("UPDATE conversations SET id=? WHERE id=?")
        .bind(&historical)
        .bind(&conversation)
        .execute(&pool)
        .await
        .unwrap();

    let initial = replay
        .snapshot(&actor, std::slice::from_ref(&historical))
        .await
        .unwrap();
    send(
        &messaging,
        &actor,
        "historical-conversation-send",
        "preserved",
    )
    .await;
    let batch = replay
        .replay(
            &actor,
            std::slice::from_ref(&historical),
            &initial.cursor,
            100,
        )
        .await
        .unwrap();
    let event = batch
        .events
        .iter()
        .find(|event| event.entity_type == "message")
        .unwrap();
    assert_eq!(event.conversation_id.as_str(), historical);
    assert_eq!(
        event.message.as_ref().unwrap().conversation_id.as_str(),
        historical
    );
    let wire = serde_json::to_value(event).unwrap();
    assert_eq!(wire["conversation_id"], historical);
    assert_eq!(wire["message"]["conversation_id"], historical);
}

#[tokio::test]
async fn cursor_is_bound_to_actor_subscription_and_database_generation() {
    let (pool, auth, actor, conversation, _, replay) = fixture().await;
    let cursor = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    let other = auth.issue_web_session("other").await.unwrap().1;
    assert!(matches!(
        replay
            .replay(&other, std::slice::from_ref(&conversation), &cursor, 100)
            .await,
        Err(ReplayError::ResyncRequired(ResyncReason::CredentialChanged))
    ));
    assert!(matches!(
        replay
            .replay(&actor, &["different".into()], &cursor, 100)
            .await,
        Err(ReplayError::ResyncRequired(
            ResyncReason::SubscriptionChanged
        ))
    ));
    sqlx::query("UPDATE database_metadata SET generation='restored' WHERE singleton=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        replay.replay(&actor, &[conversation], &cursor, 100).await,
        Err(ReplayError::ResyncRequired(ResyncReason::DatabaseRestored))
    ));
}

#[tokio::test]
async fn snapshot_then_replay_is_gap_free_and_projects_current_tombstone() {
    let (_, _, actor, conversation, messaging, replay) = fixture().await;
    let first = send(&messaging, &actor, "send-1", "first").await;
    let snapshot = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap();
    assert_eq!(snapshot.messages.len(), 1);
    let second = send(&messaging, &actor, "send-2", "second").await;
    let created = replay
        .replay(
            &actor,
            std::slice::from_ref(&conversation),
            &snapshot.cursor,
            100,
        )
        .await
        .unwrap();
    assert_eq!(created.events.len(), 1);
    assert_eq!(created.events[0].entity_id, second.message_id);

    messaging
        .delete_message(
            &actor,
            EntityCommand {
                request_id: "delete-1",
                client_message_id: "delete-1",
                operation_generation: None,
                message_id: &first.message_id,
            },
        )
        .await
        .unwrap();
    let fresh = replay.snapshot(&actor, &[conversation]).await.unwrap();
    let tombstone = fresh
        .messages
        .iter()
        .find(|message| message.message_id == first.message_id)
        .unwrap();
    assert!(tombstone.deleted);
    assert!(tombstone.content.is_none());
}

#[tokio::test]
async fn delayed_thread_tag_event_projects_current_tags_and_tag_version_together() {
    let (pool, _, actor, _, _, replay) = fixture().await;
    sqlx::query("UPDATE channels SET channel_type='forum' WHERE id='channel'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO channels( \
            id,server_id,name,channel_type,parent_channel_id,thread_tags_version \
         ) VALUES('thread','server','Thread','public_thread','channel',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO forum_tags(id,channel_id,name,position) VALUES \
         ('tag-1','channel','One',0),('tag-2','channel','Two',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='thread'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let cursor = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    let generation: String =
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut sequences = Vec::new();
    for (version, tag_id) in [(2_i64, "tag-1"), (3_i64, "tag-2")] {
        sqlx::query("DELETE FROM thread_tags WHERE thread_id='thread'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO thread_tags(thread_id,tag_id) VALUES('thread',?)")
            .bind(tag_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE channels SET thread_tags_version=? WHERE id='thread'")
            .bind(version)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version) \
             VALUES('thread_tags','thread',?) \
             ON CONFLICT(entity_type,entity_id) DO UPDATE SET version=excluded.version",
        )
        .bind(version)
        .execute(&pool)
        .await
        .unwrap();
        let sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'thread_tags_updated','thread_tags','thread',?,1,'user',?) \
             RETURNING event_sequence",
        )
        .bind(&generation)
        .bind(&conversation)
        .bind(version)
        .bind(serde_json::json!({"thread_id":"thread","tag_ids":[tag_id]}).to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        sequences.push(sequence);
    }

    let batch = replay
        .replay(&actor, std::slice::from_ref(&conversation), &cursor, 100)
        .await
        .unwrap();
    assert_eq!(batch.events.len(), 2);
    assert!(batch.events.iter().all(|event| event.entity_version == 3));
    assert!(batch.events.iter().all(|event| {
        event.descriptor == serde_json::json!({"thread_id":"thread","tag_ids":["tag-2"]})
    }));
    let (_, projected) = replay
        .project_event(&actor, sequences[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projected.entity_version, 3);
    assert_eq!(
        projected.descriptor,
        serde_json::json!({"thread_id":"thread","tag_ids":["tag-2"]})
    );
}
