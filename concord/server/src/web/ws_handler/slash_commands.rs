use super::ChatEngine;

pub(super) async fn register_slash_command(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name, description, options_json): (String, String, String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .register_slash_command(
                session_id,
                &server_id,
                &name,
                &description,
                options_json.as_deref(),
            )
            .await
    })
}

pub(super) async fn list_slash_commands(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_slash_commands(session_id, &server_id).await)
}

pub(super) async fn delete_slash_command(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (command_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.delete_slash_command(session_id, &command_id).await)
}
