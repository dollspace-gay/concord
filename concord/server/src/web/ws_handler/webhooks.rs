use super::ChatEngine;

pub(super) async fn create_webhook(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel_id, name, webhook_type, url): (
        String,
        String,
        String,
        String,
        Option<String>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_webhook(
                session_id,
                &server_id,
                &channel_id,
                &name,
                &webhook_type,
                url.as_deref(),
            )
            .await
    })
}

pub(super) async fn list_webhooks(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_webhooks(session_id, &server_id).await)
}

pub(super) async fn update_webhook(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (webhook_id, name, avatar_url, channel_id): (String, String, Option<String>, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .update_webhook(
                session_id,
                &webhook_id,
                &name,
                avatar_url.as_deref(),
                &channel_id,
            )
            .await
    })
}

pub(super) async fn delete_webhook(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (webhook_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.delete_webhook(session_id, &webhook_id).await)
}
