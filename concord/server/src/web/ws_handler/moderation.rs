use super::ChatEngine;

pub(super) async fn kick_member(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id, reason): (String, String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .kick_member(session_id, &server_id, &user_id, reason.as_deref())
            .await
    })
}

pub(super) async fn ban_member(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id, reason, delete_message_days): (String, String, Option<String>, i32),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .ban_member(
                session_id,
                &server_id,
                &user_id,
                reason.as_deref(),
                delete_message_days,
            )
            .await
    })
}

pub(super) async fn unban_member(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.unban_member(session_id, &server_id, &user_id).await)
}

pub(super) async fn list_bans(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_bans(session_id, &server_id).await)
}

pub(super) async fn timeout_member(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id, timeout_until, reason): (String, String, Option<String>, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .timeout_member(
                session_id,
                &server_id,
                &user_id,
                timeout_until.as_deref(),
                reason.as_deref(),
            )
            .await
    })
}

pub(super) async fn set_slow_mode(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, seconds): (String, String, i32),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_slowmode(session_id, &server_id, &channel, seconds)
            .await
    })
}

pub(super) async fn set_nsfw(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, is_nsfw): (String, String, bool),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_nsfw(session_id, &server_id, &channel, is_nsfw)
            .await
    })
}

pub(super) async fn bulk_delete_messages(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel, message_ids): (String, String, Vec<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .bulk_delete_messages(session_id, &server_id, &channel, message_ids)
            .await
    })
}

pub(super) async fn get_audit_log(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, action_type, limit, before): (String, Option<String>, Option<i64>, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let limit = limit.unwrap_or(50).clamp(1, 200);
        engine
            .get_audit_log(
                session_id,
                &server_id,
                action_type.as_deref(),
                limit,
                before.as_deref(),
            )
            .await
    })
}
