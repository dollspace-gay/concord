use super::{ChatEngine, ChatEvent};

pub(super) async fn typing(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, channel): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.send_typing(session_id, &server_id, &channel))
}

pub(super) async fn mark_read(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (
        operation_generation,
        request_id,
        client_message_id,
        conversation_id,
        server_id,
        channel,
        message_id,
    ): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
        String,
    ),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let fallback = uuid::Uuid::new_v4().to_string();
        let conversation_id = match conversation_id {
            Some(id) => Ok(id),
            None => {
                engine
                    .conversation_id_for_channel(&server_id, &channel)
                    .await
            }
        };
        match conversation_id {
            Ok(conversation_id) => engine
                .submit_mark_read(
                    session_id,
                    crate::engine::messaging::ReadCommand {
                        request_id: request_id.as_deref().unwrap_or(&fallback),
                        client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                        operation_generation: Some(&operation_generation),
                        conversation_id: &conversation_id,
                        message_id: &message_id,
                    },
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message())),
            Err(error) => Err(error),
        }
    })
}

pub(super) async fn get_unread_counts(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.get_unread_counts(session_id, &server_id).await {
            Ok((counts, stamps)) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send_guarded(
                        ChatEvent::UnreadCounts { server_id, counts },
                        Some(crate::engine::user_session::DeliveryGuard::Stamps(stamps)),
                    );
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
}
