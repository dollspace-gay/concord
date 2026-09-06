use super::{
    ChatEngine, ChatEvent, ClientMessage, dispatch, lifecycle_command_allowed, send_error,
    split_safe_error, warn, websocket_command_correlation,
};

pub(super) async fn handle_client_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    text: &str,
) {
    let correlation_id = websocket_command_correlation(text);
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                category = ?e.classify(),
                line = e.line(),
                column = e.column(),
                "invalid client message"
            );
            if let Some(request_id) = correlation_id
                && let Some(session) = engine.get_session(session_id)
            {
                let _ = session.send(ChatEvent::CommandError {
                    request_id,
                    code: "INVALID_INPUT".into(),
                    message: "invalid client message".into(),
                    retryable: false,
                });
            }
            return;
        }
    };

    let (msg, lifecycle_success_id) = match msg {
        ClientMessage::LifecycleCommand {
            request_id,
            command,
        } if lifecycle_command_allowed(&command) => (*command, Some(request_id)),
        ClientMessage::LifecycleCommand { request_id, .. } => {
            if let Some(session) = engine.get_session(session_id) {
                let _ = session.send(ChatEvent::CommandError {
                    request_id,
                    code: "INVALID_INPUT".into(),
                    message: "command is not a lifecycle mutation".into(),
                    retryable: false,
                });
            }
            return;
        }
        message => (message, None),
    };

    let result = match dispatch::dispatch(engine, session_id, msg).await {
        std::ops::ControlFlow::Continue(result) => result,
        std::ops::ControlFlow::Break(()) => return,
    };

    match result {
        Ok(()) => {
            if let Some(request_id) = lifecycle_success_id
                && let Some(session) = engine.get_session(session_id)
            {
                let _ = session.send(ChatEvent::LifecycleCommandSucceeded { request_id });
            }
        }
        Err(error) => {
            let (code, message) = split_safe_error(&error);
            if let Some(request_id) = correlation_id {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::CommandError {
                        request_id,
                        code: code.to_owned(),
                        message: message.to_owned(),
                        retryable: code == "DEPENDENCY_UNAVAILABLE",
                    });
                }
            } else {
                send_error(engine, session_id, code, message);
            }
        }
    }
}
