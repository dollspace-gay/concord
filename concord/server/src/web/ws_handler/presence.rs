use super::{ChatEngine, ChatEvent};

pub(super) async fn set_presence(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (status, custom_status, status_emoji): (String, Option<String>, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_presence(
                session_id,
                &status,
                custom_status.as_deref(),
                status_emoji.as_deref(),
            )
            .await
    })
}

pub(super) async fn get_presences(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_server_presences(session_id, &server_id).await {
            Ok(presences) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::PresenceList {
                        server_id,
                        presences,
                    });
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn set_server_nickname(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, nickname): (String, Option<String>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        engine
            .set_server_nickname(session_id, &server_id, nickname.as_deref())
            .await
    })
}
