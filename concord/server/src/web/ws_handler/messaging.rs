use super::ChatEngine;

pub(super) struct SendMessage {
    pub(super) operation_generation: String,
    pub(super) request_id: Option<String>,
    pub(super) client_message_id: Option<String>,
    pub(super) conversation_id: Option<String>,
    pub(super) server_id: String,
    pub(super) channel: String,
    pub(super) content: String,
    pub(super) content_format: crate::engine::messaging::ContentFormat,
    pub(super) reply_to: Option<String>,
    pub(super) attachment_ids: Option<Vec<String>>,
    pub(super) mentions: Vec<crate::engine::messaging::MessageMention>,
    pub(super) nonce: Option<String>,
}

pub(super) async fn send_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    payload: SendMessage,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    let SendMessage {
        operation_generation,
        request_id,
        client_message_id,
        conversation_id,
        server_id,
        channel,
        content,
        content_format,
        reply_to,
        attachment_ids,
        mentions,
        nonce,
    } = payload;
    std::ops::ControlFlow::Continue({
        let fallback = nonce
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let request_id = request_id.as_deref().unwrap_or(&fallback);
        let client_message_id = client_message_id.as_deref().unwrap_or(&fallback);
        engine
            .submit_channel_message(
                session_id,
                crate::engine::messaging::SendMessageCommand {
                    request_id,
                    client_message_id,
                    operation_generation: Some(&operation_generation),
                    conversation_id: conversation_id.as_deref(),
                    server_id: &server_id,
                    channel: &channel,
                    content: &content,
                    content_format,
                    reply_to_id: reply_to.as_deref(),
                    attachment_ids: attachment_ids.as_deref().unwrap_or(&[]),
                    mentions: &mentions,
                },
                nonce.as_deref(),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
    })
}

pub(super) struct SendDirectMessage {
    pub(super) operation_generation: String,
    pub(super) request_id: Option<String>,
    pub(super) client_message_id: Option<String>,
    pub(super) recipient: String,
    pub(super) content: String,
    pub(super) content_format: crate::engine::messaging::ContentFormat,
    pub(super) reply_to: Option<String>,
    pub(super) attachment_ids: Option<Vec<String>>,
    pub(super) nonce: Option<String>,
}

pub(super) async fn send_direct_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    payload: SendDirectMessage,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    let SendDirectMessage {
        operation_generation,
        request_id,
        client_message_id,
        recipient,
        content,
        content_format,
        reply_to,
        attachment_ids,
        nonce,
    } = payload;
    std::ops::ControlFlow::Continue({
        let fallback = nonce
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        engine
            .submit_direct_message(
                session_id,
                crate::engine::messaging::SendDirectMessageCommand {
                    request_id: request_id.as_deref().unwrap_or(&fallback),
                    client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                    operation_generation: Some(&operation_generation),
                    recipient: &recipient,
                    content: &content,
                    content_format,
                    reply_to_id: reply_to.as_deref(),
                    attachment_ids: attachment_ids.as_deref().unwrap_or(&[]),
                },
                nonce.as_deref(),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
    })
}

pub(super) async fn list_direct_conversations(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue(engine.list_direct_conversations(session_id).await)
}
