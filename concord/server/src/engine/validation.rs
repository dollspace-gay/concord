/// Maximum message content length (bytes).
pub const MAX_MESSAGE_LENGTH: usize = 2000;

/// Maximum channel name length.
pub const MAX_CHANNEL_NAME_LENGTH: usize = 50;

/// Maximum topic length.
pub const MAX_TOPIC_LENGTH: usize = 500;

/// Maximum server name length.
pub const MAX_SERVER_NAME_LENGTH: usize = 100;

/// Maximum nickname length.
pub const MAX_NICKNAME_LENGTH: usize = 32;
pub const MAX_DISPLAY_NAME_LENGTH: usize = 256;

/// Validate a server name. Must be 1-100 chars, non-empty after trimming.
pub fn validate_server_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Server name cannot be empty".into());
    }
    if trimmed.len() > MAX_SERVER_NAME_LENGTH {
        return Err(format!(
            "Server name too long (max {} characters)",
            MAX_SERVER_NAME_LENGTH
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err("Server name cannot contain control characters".into());
    }
    Ok(())
}

/// Validate a nickname. Must be 1-32 chars, alphanumeric + underscore/hyphen/dot.
/// Dots are allowed to support Bluesky handles (e.g., `dollspace.gay`).
pub fn validate_nickname(nick: &str) -> Result<(), String> {
    if nick.is_empty() {
        return Err("Nickname cannot be empty".into());
    }
    if nick.len() > MAX_NICKNAME_LENGTH {
        return Err(format!(
            "Nickname too long (max {} characters)",
            MAX_NICKNAME_LENGTH
        ));
    }
    if !nick
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-.[\\]^{|}~".contains(c))
    {
        return Err("Nickname contains characters unsupported by IRC".into());
    }
    let first = nick.chars().next().expect("nonempty nickname");
    if !(first.is_ascii_alphabetic() || "_[\\]^{|}~".contains(first)) {
        return Err("Nickname must start with an IRC letter or special character".into());
    }
    Ok(())
}

pub fn validate_display_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_DISPLAY_NAME_LENGTH {
        return Err("Display name must contain 1-256 bytes".into());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("Display name cannot contain control characters".into());
    }
    Ok(())
}

/// Validate a channel name. Must start with #, 2-50 chars, no spaces.
pub fn validate_channel_name(name: &str) -> Result<(), String> {
    if !name.starts_with('#') {
        return Err("Channel name must start with #".into());
    }
    if name.len() < 2 {
        return Err("Channel name too short".into());
    }
    if name.len() > MAX_CHANNEL_NAME_LENGTH {
        return Err(format!(
            "Channel name too long (max {} characters)",
            MAX_CHANNEL_NAME_LENGTH
        ));
    }
    if name
        .chars()
        .any(|character| character.is_control() || matches!(character, ' ' | ',' | ':'))
    {
        return Err("Channel name contains characters unsupported by IRC".into());
    }
    Ok(())
}

/// Sanitize user-generated content by escaping HTML entities.
/// Prevents XSS when content is rendered in the web frontend.
pub fn sanitize_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Validate message content. Must be non-empty and under the length limit.
pub fn validate_message(content: &str) -> Result<(), String> {
    validate_message_with_limit(content, MAX_MESSAGE_LENGTH)
}

/// Validate message content with a configurable length limit.
pub fn validate_message_with_limit(content: &str, max_length: usize) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("Message cannot be empty".into());
    }
    if content.len() > max_length {
        return Err(format!("Message too long (max {} characters)", max_length));
    }
    Ok(())
}

/// Validate a vanity invite code. Must be 2-32 lowercase alphanumeric + hyphens,
/// no leading/trailing hyphens.
pub fn validate_vanity_code(code: &str) -> Result<(), String> {
    if code.len() < 2 {
        return Err("Vanity code too short (min 2 characters)".into());
    }
    if code.len() > 32 {
        return Err("Vanity code too long (max 32 characters)".into());
    }
    if code.starts_with('-') || code.ends_with('-') {
        return Err("Vanity code cannot start or end with a hyphen".into());
    }
    if !code
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("Vanity code can only contain lowercase letters, digits, and hyphens".into());
    }
    Ok(())
}

/// Validate a topic string. Can be empty (to clear topic) but has a length limit.
pub fn validate_topic(topic: &str) -> Result<(), String> {
    if topic.len() > MAX_TOPIC_LENGTH {
        return Err(format!(
            "Topic too long (max {} characters)",
            MAX_TOPIC_LENGTH
        ));
    }
    if topic.chars().any(char::is_control) {
        return Err("Topic cannot contain control characters".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
