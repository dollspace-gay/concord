use super::ChatEngine;

pub(super) async fn create_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name, category_id, is_private, channel_type): (
        String,
        String,
        Option<String>,
        Option<bool>,
        Option<String>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .create_channel_in_server(
                session_id,
                &server_id,
                &name,
                category_id.as_deref(),
                is_private.unwrap_or(false),
                channel_type.as_deref().unwrap_or("text"),
            )
            .await
        {
            Ok(_) => {
                engine
                    .send_visible_channel_list(session_id, server_id)
                    .await
            }
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn delete_channel(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .delete_channel_in_server(session_id, &server_id, &channel)
            .await
        {
            Ok(()) => {
                engine
                    .send_visible_channel_list(session_id, server_id)
                    .await
            }
            Err(e) => Err(e),
        }
    })
}
