use super::{ChatEngine, ChatEvent, ClientMessage};

pub(super) fn lifecycle_command_allowed(command: &ClientMessage) -> bool {
    !matches!(
        command,
        ClientMessage::LifecycleCommand { .. }
            | ClientMessage::Sync { .. }
            | ClientMessage::SendMessage { .. }
            | ClientMessage::SendDirectMessage { .. }
            | ClientMessage::EditMessage { .. }
            | ClientMessage::DeleteMessage { .. }
            | ClientMessage::AddReaction { .. }
            | ClientMessage::RemoveReaction { .. }
            | ClientMessage::MarkRead { .. }
            | ClientMessage::InvokeSlashCommand { .. }
            | ClientMessage::InvokeMessageComponent { .. }
    )
}

pub(super) fn split_safe_error(error: &str) -> (&str, &str) {
    error
        .split_once(": ")
        .filter(|(code, _)| {
            code.chars()
                .all(|character| character.is_ascii_uppercase() || character == '_')
        })
        .unwrap_or(("COMMAND_FAILED", error))
}

pub(super) fn send_error(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    code: &str,
    message: &str,
) {
    if let Some(session) = engine.get_session(session_id) {
        let _ = session.send(ChatEvent::Error {
            code: code.into(),
            message: message.into(),
        });
    }
}
