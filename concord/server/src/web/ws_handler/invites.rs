use super::ChatEngine;

pub(super) async fn create_invite(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, max_uses, expires_at, channel_id): (
        String,
        Option<i32>,
        Option<String>,
        Option<String>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_invite(
                session_id,
                &server_id,
                max_uses,
                expires_at.as_deref(),
                channel_id.as_deref(),
            )
            .await
    })
}

pub(super) async fn list_invites(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_invites(session_id, &server_id).await)
}

pub(super) async fn delete_invite(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, invite_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .delete_invite(session_id, &server_id, &invite_id)
            .await
    })
}

pub(super) async fn use_invite(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (code,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.use_invite(session_id, &code).await)
}
