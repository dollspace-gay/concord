use super::protocol::default_server_id;
use super::*;

use tokio::sync::mpsc;

/// Helper to deserialize a JSON string into a ClientMessage.
fn parse_msg(json: &str) -> Result<ClientMessage, serde_json::Error> {
    serde_json::from_str(json)
}

async fn forum_wire_fixture(
    owner: bool,
) -> (
    ChatEngine,
    sqlx::SqlitePool,
    crate::auth::authority::AuthService,
    crate::auth::authority::CredentialId,
    crate::engine::events::ConnectionId,
    mpsc::Receiver<ChatEvent>,
) {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES \
         ('server','owner','owner'),('server','member','member')",
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
    sqlx::query(
        "INSERT INTO channels(id,server_id,name,channel_type) \
         VALUES('forum','server','#forum','forum')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let user_id = if owner { "owner" } else { "member" };
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session(user_id).await.unwrap();
    let credential_id = actor.credential_id().clone();
    let engine = ChatEngine::new(pool.clone(), auth.clone(), "replay-secret", 4000, 100);
    engine.load_servers_from_db().await.unwrap();
    engine.load_channels_from_db().await.unwrap();
    let (session_id, receiver) = engine
        .connect(
            Some(user_id.into()),
            user_id.into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    engine.bind_authenticated_actor(session_id, actor).unwrap();
    (engine, pool, auth, credential_id, session_id, receiver)
}

async fn receive_command_error(receiver: &mut mpsc::Receiver<ChatEvent>) -> (String, String, bool) {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("command response timed out")
            .expect("command response channel closed")
        {
            ChatEvent::CommandError {
                code,
                message,
                retryable,
                ..
            } => return (code, message, retryable),
            _ => continue,
        }
    }
}

async fn receive_lifecycle_success(receiver: &mut mpsc::Receiver<ChatEvent>) -> String {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("command response timed out")
            .expect("command response channel closed")
        {
            ChatEvent::LifecycleCommandSucceeded { request_id } => return request_id,
            _ => continue,
        }
    }
}

mod behavior;
mod lifecycle;
mod membership;
mod messaging;
mod queries;
mod recovery;
mod validation;
