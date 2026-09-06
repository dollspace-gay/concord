use super::{ChatEngine, ChatEvent};

pub(super) async fn sync(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (request_id, protocol_version, subscriptions, cursor, limit): (
        String,
        u32,
        Vec<String>,
        Option<String>,
        Option<usize>,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        if protocol_version != 2 {
            if let Some(session) = engine.get_session(session_id) {
                let _ = session.send(ChatEvent::ResyncRequired {
                    request_id,
                    reason: crate::engine::replay::ResyncReason::ProtocolChanged,
                });
            }
            Ok(())
        } else {
            match engine
                .synchronize(
                    session_id,
                    &subscriptions,
                    cursor.as_deref(),
                    limit.unwrap_or(100),
                )
                .await
            {
                Ok(event) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let event = match event {
                            crate::engine::chat_engine::Synchronization::Snapshot(snapshot) => {
                                ChatEvent::SyncSnapshot {
                                    request_id,
                                    snapshot,
                                }
                            }
                            crate::engine::chat_engine::Synchronization::Replay(batch) => {
                                ChatEvent::ReplayBatch { request_id, batch }
                            }
                        };
                        let _ = session.send_guarded(
                            event,
                            Some(crate::engine::user_session::DeliveryGuard::Conversations(
                                subscriptions,
                            )),
                        );
                    }
                    Ok(())
                }
                Err(crate::engine::replay::ReplayError::ResyncRequired(reason)) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::ResyncRequired { request_id, reason });
                    }
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            }
        }
    })
}
