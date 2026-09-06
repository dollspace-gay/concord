use super::ChatEngine;

pub(super) async fn pin_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, message_id): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .pin_message(session_id, &server_id, &channel, &message_id)
            .await
    })
}

pub(super) async fn unpin_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, message_id): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .unpin_message(session_id, &server_id, &channel, &message_id)
            .await
    })
}

pub(super) async fn get_pinned_messages(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .get_pinned_messages(session_id, &server_id, &channel)
            .await
    })
}
