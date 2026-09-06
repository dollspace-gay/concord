use super::{
    Digest, MAX_ATTACHMENTS, MAX_CLIENT_ID_BYTES, MAX_MENTIONS, MAX_REQUEST_ID_BYTES,
    MessagingError, SendMessageCommand, Sha256, validation,
};

pub(super) fn validate_command(
    command: &SendMessageCommand<'_>,
    max_message_length: usize,
) -> Result<(), MessagingError> {
    if command.request_id.is_empty() || command.request_id.len() > MAX_REQUEST_ID_BYTES {
        return Err(MessagingError::InvalidInput("invalid request ID".into()));
    }
    if command.client_message_id.is_empty() || command.client_message_id.len() > MAX_CLIENT_ID_BYTES
    {
        return Err(MessagingError::InvalidInput(
            "invalid client message ID".into(),
        ));
    }
    if command.attachment_ids.len() > MAX_ATTACHMENTS {
        return Err(MessagingError::InvalidInput("too many attachments".into()));
    }
    if command.mentions.len() > MAX_MENTIONS {
        return Err(MessagingError::InvalidInput("too many mentions".into()));
    }
    if command.content.is_empty() && command.attachment_ids.is_empty() {
        return Err(MessagingError::InvalidInput(
            "message content or an attachment is required".into(),
        ));
    }
    if !command.content.is_empty() {
        validation::validate_message_with_limit(command.content, max_message_length)
            .map_err(MessagingError::InvalidInput)?;
    }
    Ok(())
}

pub(super) fn validate_interaction_response_command(
    command: &SendMessageCommand<'_>,
    max_message_length: usize,
    has_rich_content: bool,
) -> Result<(), MessagingError> {
    if command.content.is_empty() && command.attachment_ids.is_empty() && has_rich_content {
        let mut validation_command = command.clone();
        validation_command.content = "interaction response";
        validate_command(&validation_command, max_message_length)
    } else {
        validate_command(command, max_message_length)
    }
}

pub(super) fn validate_operation_ids(
    request_id: &str,
    client_message_id: &str,
) -> Result<(), MessagingError> {
    if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_BYTES {
        return Err(MessagingError::InvalidInput("invalid request ID".into()));
    }
    if client_message_id.is_empty() || client_message_id.len() > MAX_CLIENT_ID_BYTES {
        return Err(MessagingError::InvalidInput(
            "invalid client message ID".into(),
        ));
    }
    Ok(())
}

pub(super) fn hash_json(value: &serde_json::Value) -> Result<String, MessagingError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| MessagingError::InvalidInput("command cannot be encoded".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn domain_tuple_id(domain: &[u8], fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn reaction_entity_id(message_id: &str, user_id: &str, emoji: &str) -> String {
    domain_tuple_id(b"concord:reaction:v1", &[message_id, user_id, emoji])
}

pub(crate) fn read_entity_id(user_id: &str, conversation_id: &str) -> String {
    domain_tuple_id(b"concord:read-state:v1", &[user_id, conversation_id])
}
