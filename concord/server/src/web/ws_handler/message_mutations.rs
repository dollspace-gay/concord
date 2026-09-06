use super::ChatEngine;

pub(super) async fn edit_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (
        operation_generation,
        request_id,
        client_message_id,
        message_id,
        content,
        content_format,
        mentions,
    ): (
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        crate::engine::messaging::ContentFormat,
        Vec<crate::engine::messaging::MessageMention>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let fallback = uuid::Uuid::new_v4().to_string();
        engine
            .submit_edit_message(
                session_id,
                crate::engine::messaging::EditMessageCommand {
                    request_id: request_id.as_deref().unwrap_or(&fallback),
                    client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                    operation_generation: Some(&operation_generation),
                    message_id: &message_id,
                    content: &content,
                    content_format,
                    mentions: &mentions,
                },
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
    })
}

pub(super) async fn delete_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (operation_generation, request_id, client_message_id, message_id): (
        String,
        Option<String>,
        Option<String>,
        String,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let fallback = uuid::Uuid::new_v4().to_string();
        engine
            .submit_delete_message(
                session_id,
                crate::engine::messaging::EntityCommand {
                    request_id: request_id.as_deref().unwrap_or(&fallback),
                    client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                    operation_generation: Some(&operation_generation),
                    message_id: &message_id,
                },
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
    })
}

pub(super) async fn add_reaction(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (operation_generation, request_id, client_message_id, message_id, emoji): (
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let fallback = uuid::Uuid::new_v4().to_string();
        engine
            .submit_reaction(
                session_id,
                crate::engine::messaging::ReactionCommand {
                    request_id: request_id.as_deref().unwrap_or(&fallback),
                    client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                    operation_generation: Some(&operation_generation),
                    message_id: &message_id,
                    emoji: &emoji,
                },
                true,
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
    })
}

pub(super) async fn remove_reaction(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (operation_generation, request_id, client_message_id, message_id, emoji): (
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let fallback = uuid::Uuid::new_v4().to_string();
        engine
            .submit_reaction(
                session_id,
                crate::engine::messaging::ReactionCommand {
                    request_id: request_id.as_deref().unwrap_or(&fallback),
                    client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                    operation_generation: Some(&operation_generation),
                    message_id: &message_id,
                    emoji: &emoji,
                },
                false,
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
    })
}
