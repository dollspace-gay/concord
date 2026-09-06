use super::*;

#[tokio::test]
async fn replay_projects_a_coherent_current_thread_state() {
    let (pool, _, actor, _, _, replay) = fixture().await;
    sqlx::query(
        "INSERT INTO channels( \
            id,server_id,name,channel_type,parent_channel_id,archived,thread_state_version \
         ) VALUES('thread','server','Thread','public_thread','channel',0,1)",
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
    for (version, archived, reason) in [(2_i64, true, Some("manual")), (3_i64, false, None)] {
        sqlx::query(
            "UPDATE channels SET archived=?,thread_state_version=?,thread_archive_reason=? \
             WHERE id='thread'",
        )
        .bind(archived)
        .bind(version)
        .bind(reason)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version) \
             VALUES('thread_state','thread',?) \
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
             ) VALUES(?,?,'thread_state_changed','thread_state','thread',?,1,'user',?) \
             RETURNING event_sequence",
        )
        .bind(&generation)
        .bind(&conversation)
        .bind(version)
        .bind(serde_json::json!({"archived": archived, "reason": reason}).to_string())
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
        event.descriptor == serde_json::json!({"archived": false, "reason": null})
    }));

    let (_, projected) = replay
        .project_event(&actor, sequences[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projected.entity_version, 3);
    assert_eq!(
        projected.descriptor,
        serde_json::json!({"archived": false, "reason": null})
    );
}

#[tokio::test]
async fn replay_projects_current_reaction_absence_after_remove_and_parent_delete() {
    let (_, _, actor, conversation, messaging, replay) = fixture().await;
    let sent = send(&messaging, &actor, "send", "message").await;
    let cursor = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    messaging
        .change_reaction(
            &actor,
            ReactionCommand {
                request_id: "add",
                client_message_id: "add",
                operation_generation: None,
                message_id: &sent.message_id,
                emoji: "heart",
            },
            true,
        )
        .await
        .unwrap();
    messaging
        .change_reaction(
            &actor,
            ReactionCommand {
                request_id: "remove",
                client_message_id: "remove",
                operation_generation: None,
                message_id: &sent.message_id,
                emoji: "heart",
            },
            false,
        )
        .await
        .unwrap();
    let batch = replay
        .replay(&actor, std::slice::from_ref(&conversation), &cursor, 100)
        .await
        .unwrap();
    let reactions: Vec<_> = batch
        .events
        .iter()
        .filter_map(|event| event.reaction.as_ref())
        .collect();
    assert_eq!(reactions.len(), 2);
    assert!(reactions.iter().all(|reaction| !reaction.present));
    assert!(
        reactions
            .iter()
            .all(|reaction| reaction.entity_version == 2)
    );

    let cursor = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    messaging
        .change_reaction(
            &actor,
            ReactionCommand {
                request_id: "add-again",
                client_message_id: "add-again",
                operation_generation: None,
                message_id: &sent.message_id,
                emoji: "heart",
            },
            true,
        )
        .await
        .unwrap();
    messaging
        .delete_message(
            &actor,
            EntityCommand {
                request_id: "delete",
                client_message_id: "delete",
                operation_generation: None,
                message_id: &sent.message_id,
            },
        )
        .await
        .unwrap();
    let batch = replay
        .replay(&actor, &[conversation], &cursor, 100)
        .await
        .unwrap();
    assert!(
        batch
            .events
            .iter()
            .filter_map(|event| event.reaction.as_ref())
            .all(|reaction| !reaction.present)
    );
}
