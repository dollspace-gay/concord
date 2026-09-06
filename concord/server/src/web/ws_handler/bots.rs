use super::ChatEngine;

pub(super) async fn create_bot(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (username, avatar_url): (String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_bot(session_id, &username, avatar_url.as_deref())
            .await
    })
}

pub(super) async fn list_owned_bots(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_owned_bots(session_id).await)
}

pub(super) async fn create_bot_token(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (bot_user_id, name, scopes): (String, String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_bot_token(session_id, &bot_user_id, &name, scopes.as_deref())
            .await
    })
}

pub(super) async fn list_bot_tokens(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (bot_user_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_bot_tokens(session_id, &bot_user_id).await)
}

pub(super) async fn delete_bot_token(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (token_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.delete_bot_token(session_id, &token_id).await)
}

pub(super) async fn add_bot_to_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, bot_user_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .add_bot_to_server(session_id, &server_id, &bot_user_id)
            .await
    })
}

pub(super) async fn remove_bot_from_server(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, bot_user_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .remove_bot_from_server(session_id, &server_id, &bot_user_id)
            .await
    })
}
