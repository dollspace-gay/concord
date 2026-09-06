use super::{ChatEngine, ChatEvent};

pub(super) async fn set_server_avatar(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, avatar_url): (String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(match engine.get_authenticated_actor(session_id) {
        Some(actor) => {
            engine
                .set_member_avatar_for_actor(&actor, &server_id, avatar_url.as_deref())
                .await
        }
        None => Err("UNAUTHENTICATED: authentication required".into()),
    })
}

pub(super) async fn set_vanity_code(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, vanity_code): (String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_vanity_code(session_id, &server_id, vanity_code.as_deref())
            .await
    })
}

pub(super) async fn get_server_limits(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        if let Some(session) = engine.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerLimits {
                max_message_length: engine.max_message_length(),
                max_file_size_mb: engine.max_file_size_mb(),
            });
        }
        Ok(())
    })
}
