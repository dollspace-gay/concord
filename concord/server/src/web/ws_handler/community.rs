use super::ChatEngine;

pub(super) async fn update_community_settings(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, description, is_discoverable, welcome_message, rules_text, category): (
        String,
        Option<String>,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .update_community_settings(
                session_id,
                &server_id,
                description.as_deref(),
                is_discoverable,
                welcome_message.as_deref(),
                rules_text.as_deref(),
                category.as_deref(),
            )
            .await
    })
}

pub(super) async fn get_community_settings(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.get_community_settings(session_id, &server_id).await)
}

pub(super) async fn discover_servers(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (category,): (Option<String>,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .discover_servers(session_id, category.as_deref())
            .await
    })
}

pub(super) async fn accept_rules(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.accept_rules(session_id, &server_id).await)
}
