use super::ChatEngine;

pub(super) async fn create_forum_tag(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, name, emoji, moderated): (String, String, String, Option<String>, bool),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_forum_tag(
                session_id,
                &server_id,
                &channel,
                &name,
                emoji.as_deref(),
                moderated,
            )
            .await
    })
}

pub(super) async fn update_forum_tag(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, tag_id, name, emoji, moderated, position): (
        String,
        String,
        String,
        String,
        Option<String>,
        bool,
        i32,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .update_forum_tag(
                session_id,
                &server_id,
                &channel,
                &tag_id,
                &name,
                emoji.as_deref(),
                moderated,
                position,
            )
            .await
    })
}

pub(super) async fn delete_forum_tag(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, tag_id): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .delete_forum_tag(session_id, &server_id, &channel, &tag_id)
            .await
    })
}

pub(super) async fn list_forum_tags(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .list_forum_tags(session_id, &server_id, &channel)
            .await
    })
}

pub(super) async fn set_thread_tags(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, thread_id, tag_ids): (String, String, Vec<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_thread_tags(session_id, &server_id, &thread_id, tag_ids)
            .await
    })
}

pub(super) async fn get_thread_tags(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, thread_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .get_thread_tags(session_id, &server_id, &thread_id)
            .await
    })
}
