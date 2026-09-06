use super::ChatEngine;

pub(super) async fn create_template(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name, description): (String, String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_template(session_id, &server_id, &name, description.as_deref())
            .await
    })
}

pub(super) async fn list_templates(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_templates(session_id, &server_id).await)
}

pub(super) async fn delete_template(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, template_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .delete_template(session_id, &server_id, &template_id)
            .await
    })
}

pub(super) async fn instantiate_template(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (template_id, server_name): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .instantiate_template(session_id, &template_id, &server_name)
            .await
    })
}
