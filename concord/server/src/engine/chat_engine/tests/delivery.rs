use super::*;

#[tokio::test]
async fn queued_server_response_is_rejected_after_ban() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('viewer','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','viewer','member')")
        .execute(&pool).await.unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("viewer").await.unwrap();
    let engine = ChatEngine::new(pool.clone(), auth, "test-replay-secret", 4000, 100);
    let (session_id, mut receiver) = engine
        .connect(
            Some("viewer".into()),
            "viewer".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(session_id, actor.clone())
        .unwrap();
    let session = engine.get_session(session_id).unwrap();
    assert!(session.send_guarded(
        ChatEvent::ChannelList {
            server_id: "server".into(),
            channels: Vec::new(),
        },
        Some(crate::engine::user_session::DeliveryGuard::ServerMembership(vec!["server".into(),])),
    ));
    receiver.recv().await.unwrap();
    let guard = session
        .take_delivery_guard()
        .expect("server response is guarded");
    assert!(engine.delivery_guard_is_current(&actor, &guard).await);

    sqlx::query(
        "INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','viewer','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(!engine.delivery_guard_is_current(&actor, &guard).await);
}

#[tokio::test]
async fn private_thread_creation_does_not_enqueue_for_unrelated_parent_viewers() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('viewer','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','viewer','member')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind(DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('parent','server','#parent')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,conversation_id,conversation_sequence) VALUES('parent-message','server','parent','owner','owner','parent',(SELECT id FROM conversations WHERE channel_id='parent'),1)")
        .execute(&pool).await.unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, owner_actor) = auth.issue_web_session("owner").await.unwrap();
    let (_, viewer_actor) = auth.issue_web_session("viewer").await.unwrap();
    let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (owner_session, mut owner_rx) = engine
        .connect(
            Some("owner".into()),
            "owner".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    let (viewer_session, mut viewer_rx) = engine
        .connect(
            Some("viewer".into()),
            "viewer".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(owner_session, owner_actor)
        .unwrap();
    engine
        .bind_authenticated_actor(viewer_session, viewer_actor)
        .unwrap();
    engine
        .join_channel(owner_session, "server", "#parent")
        .await
        .unwrap();
    engine
        .join_channel(viewer_session, "server", "#parent")
        .await
        .unwrap();
    while owner_rx.try_recv().is_ok() {}
    while viewer_rx.try_recv().is_ok() {}

    engine
        .create_thread(
            owner_session,
            "server",
            "#parent",
            "private",
            "parent-message",
            true,
        )
        .await
        .unwrap();
    assert!(matches!(
        owner_rx.try_recv(),
        Ok(ChatEvent::ThreadCreate { .. })
    ));
    assert!(viewer_rx.try_recv().is_err());
    assert!(engine.get_session(viewer_session).is_some());

    let pool = engine.get_db().unwrap();
    let thread_id: String =
        sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='parent'")
            .fetch_one(&pool)
            .await
            .unwrap();
    engine
        .archive_thread(owner_session, "server", &thread_id)
        .await
        .unwrap();
    engine
        .unarchive_thread(owner_session, "server", &thread_id)
        .await
        .unwrap();
    let state: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT archived,thread_state_version, \
            (SELECT count(*) FROM event_log WHERE entity_type='thread_state' AND entity_id=?), \
            (SELECT count(*) FROM delivery_outbox o JOIN event_log e USING(event_sequence) \
             WHERE e.entity_type='thread_state' AND e.entity_id=?) \
         FROM channels WHERE id=?",
    )
    .bind(&thread_id)
    .bind(&thread_id)
    .bind(&thread_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (0, 3, 2, 2));
    assert!(!engine.channels.get(&thread_id).unwrap().archived);
    engine.apply_thread_state_projection(&thread_id, true, 2);
    let projected = engine.channels.get(&thread_id).unwrap();
    assert!(!projected.archived);
    assert_eq!(projected.thread_state_version, 3);
}

#[tokio::test]
async fn queued_channel_response_is_rejected_after_read_history_revocation() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('viewer','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','viewer','member')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
        .bind((Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY).bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#private')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let (_, actor) = auth.issue_web_session("viewer").await.unwrap();
    let engine = ChatEngine::new(pool.clone(), auth, "replay-secret", 4000, 100);
    let guard = crate::engine::user_session::DeliveryGuard::ChannelActions(vec![(
        "channel".into(),
        crate::engine::authorization::ChannelAction::ReadHistory,
    )]);
    assert!(engine.delivery_guard_is_current(&actor, &guard).await);

    sqlx::query("UPDATE roles SET permissions=? WHERE id='everyone'")
        .bind(Permissions::VIEW_CHANNELS.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!engine.delivery_guard_is_current(&actor, &guard).await);
}

#[tokio::test]
async fn durable_dispatcher_recovers_when_immediate_projection_has_no_live_channel() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('user','carmilla')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','user','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO roles(id,server_id,name,permissions,is_default) \
         VALUES('everyone','server','@everyone',?,1)",
    )
    .bind(crate::engine::permissions::DEFAULT_EVERYONE.bits() as i64)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let (_, actor) = auth.issue_web_session("user").await.unwrap();
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth,
        "replay-secret",
        4000,
        100,
    ));
    let (session_id, mut receiver) = engine
        .connect(
            Some("user".into()),
            "carmilla".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine.bind_authenticated_actor(session_id, actor).unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let receipt = engine
        .submit_channel_message(
            session_id,
            crate::engine::messaging::SendMessageCommand {
                request_id: "request",
                client_message_id: "client",
                operation_generation: None,
                conversation_id: None,
                server_id: "server",
                channel: "#general",
                content: "durable",
                content_format: crate::engine::messaging::ContentFormat::Plain,
                reply_to_id: None,
                attachment_ids: &[],
                mentions: &[],
            },
            None,
        )
        .await
        .unwrap();
    // The immediate path had no in-memory channel projection. Restore the
    // subscription before starting the worker to model restart recovery.
    let mut channel = ChannelState::new("channel".into(), "server".into(), "#general".into());
    channel.members.insert(session_id);
    engine.channels.insert("channel".into(), channel);
    let worker = tokio::spawn(engine.clone().run_delivery_dispatcher(shutdown.clone()));
    let delivered = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = receiver.recv().await.unwrap();
            let _guard = engine
                .get_session(session_id)
                .unwrap()
                .take_delivery_guard();
            if let ChatEvent::DurableEvent { event } = event {
                break event;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(delivered.entity_id, receipt.message_id);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let completed: bool = sqlx::query_scalar(
                "SELECT completed_at IS NOT NULL FROM delivery_outbox WHERE event_sequence=?",
            )
            .bind(receipt.event_sequence_internal as i64)
            .fetch_one(&pool)
            .await
            .unwrap();
            if completed {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    worker.await.unwrap().unwrap();
}

#[tokio::test]
async fn retention_preserves_failed_gap_and_advances_replay_floor_contiguously() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('user','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    let generation: String =
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let conversation: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    for index in 1..=3 {
        let sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log(database_generation,conversation_id,event_kind,entity_type, \
                                   entity_id,entity_version,authorization_version,actor_id, \
                                   descriptor_json,created_at) \
             VALUES(?,?,'test','metadata',?,1,0,'user','{}',datetime('now','-8 days')) \
             RETURNING event_sequence",
        )
        .bind(&generation)
        .bind(&conversation)
        .bind(format!("entity-{index}"))
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO delivery_outbox(event_sequence,completed_at,last_error) \
             VALUES(?,CASE WHEN ?=2 THEN NULL ELSE datetime('now') END, \
                      CASE WHEN ?=2 THEN 'injected failure' ELSE NULL END)",
        )
        .bind(sequence)
        .bind(index)
        .bind(index)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "UPDATE event_retention_state SET dispatcher_high_water=1,retention_seconds=3600 \
         WHERE singleton=1",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let engine = ChatEngine::new(pool.clone(), auth, "replay", 4000, 100);
    assert_eq!(engine.prune_delivery_retention().await.unwrap(), 1);
    let remaining: Vec<i64> =
        sqlx::query_scalar("SELECT event_sequence FROM event_log ORDER BY event_sequence")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec![2, 3]);
    let floor: i64 = sqlx::query_scalar(
        "SELECT retained_from_sequence FROM event_retention_state WHERE singleton=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(floor, 2);
}
