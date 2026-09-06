use super::ChatEngine;

pub(super) async fn create_thread(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, parent_channel, name, message_id, is_private): (
        String,
        String,
        String,
        String,
        bool,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_thread(
                session_id,
                &server_id,
                &parent_channel,
                &name,
                &message_id,
                is_private,
            )
            .await
    })
}

pub(super) async fn archive_thread(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, thread_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .archive_thread(session_id, &server_id, &thread_id)
            .await
    })
}

pub(super) async fn unarchive_thread(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, thread_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .unarchive_thread(session_id, &server_id, &thread_id)
            .await
    })
}

pub(super) async fn list_threads(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_threads(session_id, &server_id, &channel).await)
}
