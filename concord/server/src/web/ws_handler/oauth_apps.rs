use super::ChatEngine;

pub(super) async fn create_o_auth2_app(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (name, description, redirect_uris, client_type): (String, Option<String>, Vec<String>, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_oauth2_app(
                session_id,
                &name,
                description.as_deref(),
                &redirect_uris,
                &client_type,
            )
            .await
    })
}

pub(super) async fn list_o_auth2_apps(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_oauth2_apps(session_id).await)
}

pub(super) async fn delete_o_auth2_app(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (app_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.delete_oauth2_app(session_id, &app_id).await)
}
