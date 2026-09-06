use super::ChatEngine;

pub(super) async fn set_announcement_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, is_announcement): (String, String, bool),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_announcement_channel(session_id, &server_id, &channel, is_announcement)
            .await
    })
}

pub(super) async fn follow_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (source_channel_id, target_channel_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .follow_channel(session_id, &source_channel_id, &target_channel_id)
            .await
    })
}

pub(super) async fn unfollow_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (follow_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.unfollow_channel(session_id, &follow_id).await)
}

pub(super) async fn list_channel_follows(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (channel_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_channel_follows(session_id, &channel_id).await)
}

pub(super) async fn publish_announcement(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (message_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.publish_announcement(session_id, &message_id).await)
}
