use super::{ChatEngine, ChatEvent};

pub(super) async fn invoke_slash_command(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (request_id, server_id, channel, command_name, args_json): (
        String,
        String,
        String,
        String,
        Option<String>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let result = engine
            .invoke_slash_command(
                session_id,
                &server_id,
                &channel,
                &command_name,
                args_json.as_deref(),
            )
            .await;
        if result.is_ok()
            && let Some(session) = engine.get_session(session_id)
        {
            let _ = session.send(ChatEvent::InteractionInvoked { request_id });
        }
        result
    })
}

pub(super) async fn invoke_message_component(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (request_id, message_id, custom_id, values): (String, String, String, Vec<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let result = engine
            .invoke_message_component(session_id, &message_id, &custom_id, &values)
            .await;
        if result.is_ok()
            && let Some(session) = engine.get_session(session_id)
        {
            let _ = session.send(ChatEvent::InteractionInvoked { request_id });
        }
        result
    })
}

pub(super) async fn respond_to_interaction(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (interaction_id, content, embeds_json, components_json, ephemeral): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .respond_to_interaction(
                session_id,
                &interaction_id,
                content.as_deref(),
                embeds_json.as_deref(),
                components_json.as_deref(),
                ephemeral.unwrap_or(false),
            )
            .await
    })
}
