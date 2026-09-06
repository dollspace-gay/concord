use sqlx::SqlitePool;

use uuid::Uuid;

use crate::db::models::{
    CreateAuditLogParams, CreateAutomodRuleParams, CreateServerEventParams, CreateWebhookParams,
};

use crate::db::pool::{create_pool, current_schema_version, run_migrations};

use crate::db::queries;

use crate::engine::chat_engine::ChatEngine;

use crate::engine::events::ChatEvent;

use crate::engine::ids::ConnectionId;

use crate::engine::permissions::{
    ChannelOverride, DEFAULT_EVERYONE, DEFAULT_MODERATOR, OverrideTargetType, Permissions,
    compute_effective_permissions,
};

use crate::engine::user_session::Protocol;

/// Create an in-memory SQLite pool with all migrations applied.
async fn setup_db() -> SqlitePool {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

/// Create a ChatEngine backed by a fresh in-memory database.
async fn setup_engine() -> (ChatEngine, SqlitePool) {
    let pool = setup_db().await;
    let auth =
        crate::auth::authority::AuthService::new(pool.clone(), "integration-secret".into(), 1);
    let engine = ChatEngine::new(pool.clone(), auth, "session-secret", 4000, 100);
    (engine, pool)
}

/// Create a test user in the database and return the user_id.
async fn create_test_user(pool: &SqlitePool, username: &str) -> String {
    let user_id = Uuid::new_v4().to_string();
    queries::users::create_with_oauth(
        pool,
        &queries::users::CreateOAuthUser {
            user_id: &user_id,
            username,
            email: Some(&format!("{username}@test.com")),
            avatar_url: None,
            oauth_id: &Uuid::new_v4().to_string(),
            provider: "github",
            provider_id: &Uuid::new_v4().to_string(),
        },
    )
    .await
    .unwrap();
    user_id
}

/// Connect a user to the engine and return (session_id, receiver).
fn connect_user(
    engine: &ChatEngine,
    user_id: Option<&str>,
    nickname: &str,
) -> (ConnectionId, tokio::sync::mpsc::Receiver<ChatEvent>) {
    engine
        .connect(
            user_id.map(|s| s.to_string()),
            nickname.to_string(),
            Protocol::WebSocket,
            None,
        )
        .unwrap()
}

async fn authenticate_session(
    engine: &ChatEngine,
    pool: &SqlitePool,
    user_id: &str,
    session_id: ConnectionId,
) {
    let auth =
        crate::auth::authority::AuthService::new(pool.clone(), "integration-secret".into(), 1);
    let (_, actor) = auth.issue_web_session(user_id).await.unwrap();
    engine.bind_authenticated_actor(session_id, actor).unwrap();
}

async fn actor_for(pool: &SqlitePool, user_id: &str) -> crate::auth::authority::Actor {
    let auth =
        crate::auth::authority::AuthService::new(pool.clone(), "integration-secret".into(), 1);
    auth.issue_web_session(user_id).await.unwrap().1
}

/// Drain all pending events from a receiver.
fn drain_events(rx: &mut tokio::sync::mpsc::Receiver<ChatEvent>) {
    while rx.try_recv().is_ok() {}
}

mod all_migrations_apply_cleanly;
mod authorization;
mod channel_override_persistence;
mod forum_channel_with_tags;
mod full_user_registration_to_server_creation_flow;
mod lifecycle;
mod message_pinning_flow;
mod message_send_edit_delete_lifecycle;
mod query_cases;
mod revocation;
mod rules_acceptance;
