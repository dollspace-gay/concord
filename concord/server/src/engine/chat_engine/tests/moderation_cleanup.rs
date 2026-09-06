use super::*;

#[tokio::test]
async fn ban_cleanup_advances_in_canonical_restart_safe_batches() {
    let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
    for index in 0..102 {
        insert_moderation_message(&pool, &format!("ban-message-{index:03}")).await;
    }
    let conversation_id: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO attachments( \
            id,uploader_id,message_id,filename,original_filename,content_type,file_size, \
            conversation_id,media_state,storage_backend,storage_key,reserved_bytes \
         ) VALUES('ban-attachment','target','ban-message-000','file','file', \
            'text/plain',4,?,'attached','local','ban-key',4)",
    )
    .bind(&conversation_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('announcement-server','Announcements','moderator')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('announcement-server','moderator','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('announcement-channel','announcement-server','#announcements')")
        .execute(&pool)
        .await
        .unwrap();
    let announcement_conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='announcement-channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO messages( \
            id,server_id,channel_id,sender_id,sender_nick,content, \
            conversation_id,conversation_sequence \
         ) VALUES('announcement-copy','announcement-server','announcement-channel', \
            'target','target','copy',?,1)",
    )
    .bind(&announcement_conversation)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhooks( \
            id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
         ) VALUES('announcement-delete-hook','announcement-server','announcement-channel', \
            'Announcement Hook','outgoing','announcement-delete-token', \
            'https://example.com/announcement-hook','moderator','active')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhook_events(id,webhook_id,event_type) \
         VALUES('announcement-delete-subscription','announcement-delete-hook','message_delete')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO announcement_publications( \
            id,follow_id,source_message_id,target_message_id,source_version,state \
         ) VALUES('publication','historical-follow','ban-message-000', \
            'announcement-copy',1,'published')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhooks( \
            id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
         ) VALUES('delete-hook','server','channel','Hook','outgoing','delete-token', \
            'https://example.com/hook','moderator','active')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhook_events(id,webhook_id,event_type) \
         VALUES('delete-subscription','delete-hook','message_delete')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE conversations SET next_message_sequence=( \
            SELECT COALESCE(MAX(conversation_sequence),0)+1 FROM messages \
            WHERE conversation_id=conversations.id \
         ) WHERE id=?",
    )
    .bind(&conversation_id)
    .execute(&pool)
    .await
    .unwrap();
    engine
        .ban_member(moderator_session, "server", "target", None, 7)
        .await
        .unwrap();
    engine
        .unban_member(moderator_session, "server", "target")
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id='ban-message-101'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO messages( \
            id,server_id,channel_id,sender_id,sender_nick,content, \
            conversation_id,conversation_sequence \
         ) SELECT 'post-unban-message','server','channel','target','target','new', \
            id,next_message_sequence FROM conversations WHERE id=?",
    )
    .bind(&conversation_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE conversations SET next_message_sequence=next_message_sequence+1 WHERE id=?",
    )
    .bind(&conversation_id)
    .execute(&pool)
    .await
    .unwrap();

    let scheduled: (String, i64, i64) = sqlx::query_as(
        "SELECT state,deleted_count, \
            (SELECT count(*) FROM messages WHERE server_id='server' \
             AND sender_id='target' AND deleted_at IS NULL) \
         FROM moderation_cleanup_jobs WHERE server_id='server' AND user_id='target'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(scheduled, ("pending".into(), 0, 102));

    assert_eq!(
        engine.process_moderation_cleanup_batch().await.unwrap(),
        100
    );
    let first: (String, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT state,deleted_count, \
            (SELECT count(*) FROM messages WHERE server_id='server' \
             AND sender_id='target' AND deleted_at IS NOT NULL), \
            (SELECT count(*) FROM entity_versions WHERE entity_type='message' \
             AND entity_id LIKE 'ban-message-%' AND version=2), \
            (SELECT count(*) FROM event_log e JOIN delivery_outbox o USING(event_sequence) \
             WHERE e.event_kind='message_deleted' AND e.entity_id LIKE 'ban-message-%') \
         FROM moderation_cleanup_jobs WHERE server_id='server' AND user_id='target'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first, ("pending".into(), 100, 100, 100, 100));

    assert_eq!(engine.process_moderation_cleanup_batch().await.unwrap(), 1);
    let completed: (String, i64, i64) = sqlx::query_as(
        "SELECT state,deleted_count, \
            (SELECT count(*) FROM messages WHERE server_id='server' \
             AND sender_id='target' AND deleted_at IS NOT NULL) \
         FROM moderation_cleanup_jobs WHERE server_id='server' AND user_id='target'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(completed, ("completed".into(), 101, 101));
    let canonical_effects: (String, i64, String, i64) = sqlx::query_as(
        "SELECT media_state, \
            (SELECT deleted_at IS NOT NULL FROM messages WHERE id='announcement-copy'), \
            (SELECT state FROM announcement_publications WHERE id='publication'), \
            (SELECT count(*) FROM webhook_deliveries WHERE event_type='message_delete') \
         FROM attachments WHERE id='ban-attachment'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        canonical_effects,
        ("deleting".into(), 1, "deleted".into(), 102)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM messages WHERE id='post-unban-message' AND deleted_at IS NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(engine.process_moderation_cleanup_batch().await.unwrap(), 0);
}
