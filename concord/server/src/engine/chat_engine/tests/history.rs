use super::*;

#[test]
fn persisted_timestamps_accept_current_and_legacy_sqlite_forms_exactly() {
    assert_eq!(
        parse_persisted_timestamp("2024-01-02 03:04:05.123456")
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        "2024-01-02T03:04:05.123456Z"
    );
    assert_eq!(
        parse_persisted_timestamp("2024-01-02T03:04:05.123456+00:00")
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        "2024-01-02T03:04:05.123456Z"
    );
    assert!(parse_persisted_timestamp("not-a-timestamp").is_none());
}

#[tokio::test]
async fn history_reload_and_snapshot_preserve_legacy_and_current_timestamps() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')")
        .execute(&pool)
        .await
        .unwrap();
    let conversation_id: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let legacy_id = format!(" historical:旧消息/{} ", "界".repeat(512));
    let current_id = "10000000-0000-0000-0000-000000000002";
    sqlx::query(
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at,edited_at,conversation_id,conversation_sequence,content_format) VALUES \
         (?, 'server','channel','owner','owner','legacy','2024-01-02 03:04:05.123456','2024-01-02 04:05:06.654321',?,1,'plain'), \
         (?, 'server','channel','owner','owner','current','2024-01-03T03:04:05.123456+00:00',NULL,?,2,'plain')",
    )
    .bind(&legacy_id)
    .bind(&conversation_id)
    .bind(current_id)
    .bind(&conversation_id)
    .execute(&pool)
    .await
    .unwrap();

    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("owner").await.unwrap().1;
    let engine = ChatEngine::new(pool.clone(), auth, "replay-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let first = engine
        .fetch_history("server", "#general", None, 50, &actor)
        .await
        .unwrap()
        .0;
    let first_wire = serde_json::to_value(&first).unwrap();
    assert_eq!(first_wire[0]["id"], current_id);
    assert_eq!(first_wire[0]["timestamp"], "2024-01-03T03:04:05.123456Z");
    assert_eq!(first_wire[1]["id"], legacy_id);
    assert_eq!(first_wire[1]["timestamp"], "2024-01-02T03:04:05.123456Z");
    assert_eq!(first_wire[1]["edited_at"], "2024-01-02T04:05:06.654321Z");

    let snapshot = engine
        .replay_service()
        .snapshot(&actor, std::slice::from_ref(&conversation_id))
        .await
        .unwrap();
    assert_eq!(snapshot.messages[0].message_id, legacy_id);
    assert_eq!(
        snapshot.messages[0].created_at,
        "2024-01-02 03:04:05.123456"
    );
    assert_eq!(snapshot.messages[1].message_id, current_id);
    assert_eq!(
        snapshot.messages[1].created_at,
        "2024-01-03T03:04:05.123456+00:00"
    );

    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("owner").await.unwrap().1;
    let reloaded = ChatEngine::new(pool, auth, "replay-secret", 4000, 100);
    reloaded.load_servers_from_db().await.unwrap();
    reloaded.load_channels_from_db().await.unwrap();
    let second = reloaded
        .fetch_history("server", "#general", None, 50, &actor)
        .await
        .unwrap()
        .0;
    assert_eq!(serde_json::to_value(second).unwrap(), first_wire);
}
