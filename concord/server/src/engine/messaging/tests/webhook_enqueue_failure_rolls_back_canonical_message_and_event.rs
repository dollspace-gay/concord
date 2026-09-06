use super::*;

#[tokio::test]
async fn webhook_enqueue_failure_rolls_back_canonical_message_and_event() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query(
        "INSERT INTO webhooks( \
            id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
         ) VALUES('hook','server','channel','Hook','outgoing','hash', \
            'https://example.com/hook','user','active')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO webhook_events(id,webhook_id,event_type) \
         VALUES('subscription','hook','message_create')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_webhook_job BEFORE INSERT ON external_jobs \
         WHEN NEW.operation_type='webhook_delivery' \
         BEGIN SELECT RAISE(FAIL,'injected'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await,
        Err(MessagingError::Internal(_))
    ));
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM messages), \
                (SELECT COUNT(*) FROM event_log), \
                (SELECT COUNT(*) FROM external_jobs), \
                (SELECT COUNT(*) FROM webhook_deliveries)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
}

#[tokio::test]
async fn announcement_lineage_survives_reopen_without_duplicate_destination() {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!("sqlite://{}", directory.path().join("concord.db").display());
    let pool = create_pool(&database_url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('user','carmilla')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('user','user','canonical_id')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Source','user'),('target-server','Target','user')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','user','owner'),('target-server','user','owner')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind(DEFAULT_EVERYONE.bits() as i64).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name,is_announcement) VALUES('channel','server','#general',1),('target-channel','target-server','#news',0)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channel_follows(id,source_channel_id,target_channel_id,created_by) VALUES('follow','channel','target-channel','user')")
        .execute(&pool).await.unwrap();
    let auth = AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("user").await.unwrap().1;
    let service = MessagingService::new(pool.clone(), auth, 4000);
    let source = service
        .send_channel_message(&actor, command("send", "client", "original"))
        .await
        .unwrap();
    let published = service
        .publish_announcement(
            &actor,
            PublishAnnouncementCommand {
                message_id: &source.message_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(published.len(), 1);
    let target_message_id = published[0].target_message_id.clone();
    drop(service);
    pool.close().await;

    let pool = create_pool(&database_url).await.unwrap();
    let auth = AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("user").await.unwrap().1;
    let service = MessagingService::new(pool.clone(), auth, 4000);
    let replay = service
        .publish_announcement(
            &actor,
            PublishAnnouncementCommand {
                message_id: &source.message_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(replay[0].target_message_id, target_message_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM announcement_publications WHERE source_message_id=?"
        )
        .bind(&source.message_id)
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
                request_id: "edit",
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
            .bind(&target_message_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "corrected"
    );
    drop(service);
    pool.close().await;

    let pool = create_pool(&database_url).await.unwrap();
    let auth = AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("user").await.unwrap().1;
    let service = MessagingService::new(pool.clone(), auth, 4000);
    service
        .delete_message(
            &actor,
            EntityCommand {
                request_id: "delete",
                client_message_id: "delete-client",
                operation_generation: None,
                message_id: &source.message_id,
            },
        )
        .await
        .unwrap();
    let state: (String, i64, i64, i64) = sqlx::query_as("SELECT ap.state,ap.source_version,(m.deleted_at IS NOT NULL),(SELECT count(*) FROM announcement_publications WHERE source_message_id=?) FROM announcement_publications ap JOIN messages m ON m.id=ap.target_message_id WHERE ap.source_message_id=?")
        .bind(&source.message_id).bind(&source.message_id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("deleted".into(), 3, 1, 1));
}

#[tokio::test]
async fn identical_retry_returns_canonical_receipt_and_conflict_is_rejected() {
    let (pool, _, actor, service) = fixture().await;
    let original = service
        .send_channel_message(&actor, command("request-1", "client", "hello"))
        .await
        .unwrap();
    let retry = service
        .send_channel_message(&actor, command("request-2", "client", "hello"))
        .await
        .unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.request_id, "request-2");
    assert_eq!(retry.message_id, original.message_id);
    assert_eq!(retry.sequence, original.sequence);
    assert!(matches!(
        service
            .send_channel_message(&actor, command("request-3", "client", "different"))
            .await,
        Err(MessagingError::IdempotencyConflict)
    ));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn forced_event_failure_rolls_back_message_sequence_and_receipt() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query(
        "CREATE TRIGGER fail_event BEFORE INSERT ON event_log \
         BEGIN SELECT RAISE(ABORT,'forced event failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let error = service
        .send_channel_message(&actor, command("request", "client", "hello"))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, MessagingError::Internal(source) if source.to_string().contains("forced event failure")),
        "fault injection did not reach event insertion: {error:?}"
    );
    let state: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT COUNT(*) FROM messages), \
            (SELECT COUNT(*) FROM command_receipts), \
            (SELECT next_message_sequence FROM conversations WHERE channel_id='channel')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 0, 0));
}

#[tokio::test]
async fn interaction_response_rolls_back_consumption_when_message_event_fails() {
    let (pool, auth, actor, service) = fixture().await;
    sqlx::query(
        "INSERT INTO interactions
         (id,interaction_type,user_id,server_id,channel_id,data_json,
          application_user_id,expires_at,response_state)
         VALUES('interaction','slash_command','other','server','channel','{}',
                'user',datetime('now','+5 minutes'),'pending')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_interaction_event BEFORE INSERT ON event_log
         BEGIN SELECT RAISE(ABORT,'forced interaction event failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = service
        .respond_to_interaction_public(
            &actor,
            "interaction",
            command(
                "interaction-request",
                "interaction:interaction:response:1",
                "hello",
            ),
            Some(r##"[{"title":"Result","url":"https://example.test/result","color":"#5865f2"}]"##),
            Some(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, MessagingError::Internal(source) if source.to_string().contains("forced interaction event failure"))
    );
    let state: (String, i64, i64) = sqlx::query_as(
        "SELECT response_state,
                (SELECT COUNT(*) FROM messages),
                (SELECT COUNT(*) FROM command_receipts)
         FROM interactions WHERE id='interaction'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("pending".into(), 0, 0));

    sqlx::query("DROP TRIGGER fail_interaction_event")
        .execute(&pool)
        .await
        .unwrap();
    let receipt = service
        .respond_to_interaction_public(
            &actor,
            "interaction",
            command(
                "interaction-retry",
                "interaction:interaction:response:1",
                "hello",
            ),
            Some(r##"[{"title":"Result","url":"https://example.test/result","color":"#5865f2"}]"##),
            Some(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#),
        )
        .await
        .unwrap();
    let committed: (String, String, String, String) = sqlx::query_as(
        "SELECT i.response_state,i.response_message_id,m.rich_embeds_json,m.components_json
         FROM interactions i JOIN messages m ON m.id=i.response_message_id
         WHERE i.id='interaction'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(committed.0, "responded");
    assert_eq!(committed.1, receipt.message_id);
    assert!(committed.2.contains("Result"));
    assert!(committed.3.contains("confirm"));
    let conversation_id: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let snapshot = crate::engine::replay::ReplayService::new(pool, auth, "replay-secret")
        .snapshot(&actor, &[conversation_id])
        .await
        .unwrap();
    let projected = snapshot
        .messages
        .iter()
        .find(|message| message.message_id == receipt.message_id)
        .unwrap();
    assert_eq!(
        projected.rich_embeds.as_ref().unwrap()[0].title.as_deref(),
        Some("Result")
    );
    assert!(matches!(
        projected.components.as_ref().unwrap()[0],
        crate::engine::events::MessageComponent::ActionRow { .. }
    ));
}

#[tokio::test]
async fn forced_receipt_failure_rolls_back_message_event_and_outbox() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query(
        "CREATE TRIGGER fail_receipt BEFORE INSERT ON command_receipts \
         BEGIN SELECT RAISE(ABORT,'forced receipt failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await,
        Err(MessagingError::Internal(_))
    ));
    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM messages), \
                (SELECT COUNT(*) FROM event_log), \
                (SELECT COUNT(*) FROM delivery_outbox), \
                (SELECT next_message_sequence FROM conversations WHERE channel_id='channel')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 0, 0, 0));
}

#[tokio::test]
async fn forced_attachment_link_failure_rolls_back_message_and_claim() {
    let (pool, _, actor, service) = fixture().await;
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO attachments( \
             id,uploader_id,filename,original_filename,content_type,file_size,conversation_id, \
             media_state,storage_backend,storage_key,reserved_bytes \
         ) VALUES('attachment','user','file','file','text/plain',4,?,'ready','local','key',4)",
    )
    .bind(conversation)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_attachment BEFORE UPDATE ON attachments \
         WHEN NEW.media_state='attached' \
         BEGIN SELECT RAISE(ABORT,'forced attachment link failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let attachments = vec!["attachment".to_owned()];
    let mut send = command("request", "client", "hello");
    send.attachment_ids = &attachments;
    assert!(matches!(
        service.send_channel_message(&actor, send).await,
        Err(MessagingError::Internal(_))
    ));
    let state: (i64, Option<String>, String) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM messages),message_id,media_state \
         FROM attachments WHERE id='attachment'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, None, "ready".into()));
}

#[tokio::test]
async fn locked_database_timeout_cancels_cleanly_and_later_send_succeeds() {
    let before = crate::runtime_metrics::snapshot();
    let database_index = crate::runtime_metrics::Operation::DatabaseWrite as usize;
    let (pool, _, actor, service) = fixture().await;
    let lock = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result = service
        .send_channel_message(&actor, command("request", "client", "hello"))
        .await;
    assert!(matches!(result, Err(MessagingError::DependencyUnavailable)));
    lock.rollback().await.unwrap();
    let receipt = service
        .send_channel_message(&actor, command("request", "client", "hello"))
        .await
        .unwrap();
    assert_eq!(receipt.sequence, "1");
    let after = crate::runtime_metrics::snapshot();
    assert!(after.failed[database_index] > before.failed[database_index]);
    assert!(after.succeeded[database_index] > before.succeeded[database_index]);
}

#[tokio::test]
async fn direct_send_block_rolls_back_new_conversation_and_message() {
    let (pool, _, actor, service) = fixture().await;
    sqlx::query("INSERT INTO user_blocks(blocker_user_id,blocked_user_id) VALUES('other','user')")
        .execute(&pool)
        .await
        .unwrap();
    let result = service
        .send_direct_message(
            &actor,
            SendDirectMessageCommand {
                request_id: "direct",
                client_message_id: "direct-client",
                operation_generation: None,
                recipient: "other",
                content: "hello",
                content_format: ContentFormat::Plain,
                reply_to_id: None,
                attachment_ids: &[],
            },
        )
        .await;
    assert!(matches!(result, Err(MessagingError::Unavailable)));
    let state: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM conversations WHERE kind='direct'), \
                (SELECT COUNT(*) FROM direct_conversation_pairs), \
                (SELECT COUNT(*) FROM messages WHERE channel_id IS NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 0, 0));
}
