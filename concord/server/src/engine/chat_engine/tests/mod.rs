use super::*;

pub(super) async fn moderation_engine_fixture()
-> (ChatEngine, SqlitePool, ConnectionId, ConnectionId) {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users(id,username) VALUES \
         ('owner','owner'),('moderator','moderator'),('target','target')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('server','owner','owner'),('server','moderator','member'), \
         ('server','target','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO roles(id,server_id,name,position,permissions,is_default) VALUES \
         ('everyone','server','@everyone',0,?,1), \
         ('moderator-role','server','Moderator',10,?,0), \
         ('target-role','server','Target',1,0,0)",
    )
    .bind(DEFAULT_EVERYONE.bits() as i64)
    .bind(
        (Permissions::KICK_MEMBERS
            | Permissions::BAN_MEMBERS
            | Permissions::MANAGE_MESSAGES
            | Permissions::MANAGE_CHANNELS
            | Permissions::MANAGE_SERVER)
            .bits() as i64,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_roles(server_id,user_id,role_id) VALUES \
         ('server','moderator','moderator-role'),('server','target','target-role')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channels(id,server_id,name,channel_type) \
         VALUES('channel','server','#general','forum')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let moderator_actor = auth.issue_web_session("moderator").await.unwrap().1;
    let target_actor = auth.issue_web_session("target").await.unwrap().1;
    let engine = ChatEngine::new(pool.clone(), auth, "replay-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (moderator_session, _) = engine
        .connect(
            Some("moderator".into()),
            "moderator".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(moderator_session, moderator_actor)
        .unwrap();
    let (target_session, _) = engine
        .connect(
            Some("target".into()),
            "target".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine
        .bind_authenticated_actor(target_session, target_actor)
        .unwrap();
    engine
        .join_channel(target_session, "server", "#general")
        .await
        .unwrap();
    (engine, pool, moderator_session, target_session)
}

pub(super) async fn insert_moderation_message(pool: &SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO messages( \
            id,server_id,channel_id,sender_id,sender_nick,content, \
            conversation_id,conversation_sequence \
         ) VALUES( \
            ?,'server','channel','target','target','message', \
            (SELECT id FROM conversations WHERE channel_id='channel'), \
            (SELECT COALESCE(MAX(conversation_sequence),0)+1 FROM messages \
             WHERE channel_id='channel') \
         )",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

/// Helper: create engine with a default server in memory (no DB).
pub(super) fn setup_engine() -> ChatEngine {
    let engine = ChatEngine::test_harness(4000, 100);
    let mut state = ServerState::new(
        DEFAULT_SERVER_ID.to_string(),
        "Concord".to_string(),
        "system".to_string(),
        None,
    );
    for (id, name) in [("test-general", "#general"), ("test-rust", "#rust")] {
        state.channel_ids.insert(id.to_string());
        engine.channels.insert(
            id.to_string(),
            ChannelState::new(
                id.to_string(),
                DEFAULT_SERVER_ID.to_string(),
                name.to_string(),
            ),
        );
        engine.channel_name_index.insert(
            (DEFAULT_SERVER_ID.to_string(), name.to_string()),
            id.to_string(),
        );
    }
    engine.servers.insert(DEFAULT_SERVER_ID.to_string(), state);
    engine
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn authenticated_credential_tracks_all_connections_until_final_disconnect() {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('user-1','alice')")
        .execute(&pool)
        .await
        .unwrap();
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("user-1").await.unwrap();
    let credential_id = actor.credential_id().clone();
    let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);
    let (sid1, _) = engine
        .connect(
            Some("user-1".into()),
            "alice".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    let (sid2, _) = engine
        .connect(Some("user-1".into()), "alice".into(), Protocol::Irc, None)
        .unwrap();

    engine
        .bind_authenticated_actor(sid1, actor.clone())
        .unwrap();
    engine.bind_authenticated_actor(sid2, actor).unwrap();
    assert_eq!(
        engine
            .credential_connections
            .get(&credential_id)
            .unwrap()
            .len(),
        2
    );

    engine.disconnect(sid1);
    assert_eq!(
        engine
            .credential_connections
            .get(&credential_id)
            .unwrap()
            .len(),
        1
    );
    engine.disconnect(sid2);
    assert!(engine.credential_connections.get(&credential_id).is_none());
}

mod channels;
mod connections;
mod delivery;
mod forum_tags;
mod history;
mod interactions;
mod messages;
mod moderation;
mod moderation_cleanup;
mod profiles;
mod search;
mod servers;
