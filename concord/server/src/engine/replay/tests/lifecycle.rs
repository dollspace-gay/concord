use super::*;

#[tokio::test]
async fn legacy_reply_projection_never_crosses_conversations_and_redacts_deleted_targets() {
    let (pool, _, actor, conversation, _, replay) = fixture().await;
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('private','server','#private')")
        .execute(&pool)
        .await
        .unwrap();
    let private_conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='private'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
                              conversation_id,conversation_sequence,content_format) \
         VALUES('private-target','server','private','other','laura','private text',?,1,'plain'), \
               ('public-reply','server','channel','user','carmilla','public reply',?,1,'plain')",
    )
    .bind(&private_conversation)
    .bind(&conversation)
    .execute(&pool)
    .await
    .unwrap();
    // Simulate a historical row created before same-conversation reply validation.
    sqlx::query("UPDATE messages SET reply_to_id='private-target' WHERE id='public-reply'")
        .execute(&pool)
        .await
        .unwrap();

    let snapshot = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap();
    let public_reply = snapshot
        .messages
        .iter()
        .find(|message| message.message_id == "public-reply")
        .unwrap();
    assert!(public_reply.reply_to_id.is_none());
    assert!(public_reply.reply_to.is_none());

    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,deleted_at, \
                              conversation_id,conversation_sequence,content_format) \
         VALUES('deleted-target','server','channel','other','laura','deleted secret',datetime('now'),?,2,'plain'), \
               ('deleted-reply','server','channel','user','carmilla','same conversation',NULL,?,3,'plain')",
    )
    .bind(&conversation)
    .bind(&conversation)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE messages SET reply_to_id='deleted-target' WHERE id='deleted-reply'")
        .execute(&pool)
        .await
        .unwrap();
    let snapshot = replay.snapshot(&actor, &[conversation]).await.unwrap();
    let deleted_reply = snapshot
        .messages
        .iter()
        .find(|message| message.message_id == "deleted-reply")
        .unwrap()
        .reply_to
        .as_ref()
        .unwrap();
    assert!(deleted_reply.deleted);
    assert!(deleted_reply.content.is_none());
}

#[tokio::test]
async fn unrelated_events_do_not_create_observable_empty_pages() {
    let (pool, auth, actor, conversation, _messaging, replay) = fixture().await;
    let cursor = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap()
        .cursor;
    let _other = auth.issue_web_session("other").await.unwrap().1;
    sqlx::query(
        "INSERT INTO channels(id,server_id,name) VALUES('other-channel','server','#other')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let generation: String =
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let other_conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='other-channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<1000) \
         INSERT INTO event_log(database_generation,conversation_id,event_kind,entity_type, \
                               entity_id,entity_version,authorization_version,actor_id,descriptor_json) \
         SELECT ?,?,'unrelated','metadata','noise-' || n,1,0,'other','{}' FROM numbers",
    )
    .bind(generation)
    .bind(other_conversation)
    .execute(&pool)
    .await
    .unwrap();
    let batch = replay
        .replay(&actor, &[conversation], &cursor, 1)
        .await
        .unwrap();
    assert!(batch.events.is_empty());
    assert!(!batch.has_more);
}
