use super::ChatEngine;

pub(super) async fn create_automod_rule(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name, rule_type, config, action_type, timeout_duration_seconds): (
        String,
        String,
        String,
        String,
        String,
        Option<i32>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .create_automod_rule(
                session_id,
                &crate::engine::chat_engine::CreateAutomodRuleRequest {
                    server_id: &server_id,
                    name: &name,
                    rule_type: &rule_type,
                    config: &config,
                    action_type: &action_type,
                    timeout_duration_seconds,
                },
            )
            .await
    })
}

pub(super) async fn update_automod_rule(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, rule_id, name, enabled, config, action_type, timeout_duration_seconds): (
        String,
        String,
        String,
        bool,
        String,
        String,
        Option<i32>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .update_automod_rule(
                session_id,
                &crate::engine::chat_engine::UpdateAutomodRuleRequest {
                    rule_id: &rule_id,
                    server_id: &server_id,
                    name: &name,
                    enabled,
                    config: &config,
                    action_type: &action_type,
                    timeout_duration_seconds,
                },
            )
            .await
    })
}

pub(super) async fn delete_automod_rule(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, rule_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .delete_automod_rule(session_id, &server_id, &rule_id)
            .await
    })
}

pub(super) async fn list_automod_rules(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_automod_rules(session_id, &server_id).await)
}
