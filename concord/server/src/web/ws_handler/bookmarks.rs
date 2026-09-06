use super::ChatEngine;

pub(super) async fn add_bookmark(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (message_id, note): (String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .add_bookmark(session_id, &message_id, note.as_deref())
            .await
    })
}

pub(super) async fn remove_bookmark(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (message_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.remove_bookmark(session_id, &message_id).await)
}

pub(super) async fn list_bookmarks(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_bookmarks(session_id).await)
}
