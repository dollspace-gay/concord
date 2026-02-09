/// Maximum message content length (bytes).
pub const MAX_MESSAGE_LENGTH: usize = 2000;

/// Maximum channel name length.
pub const MAX_CHANNEL_NAME_LENGTH: usize = 50;

/// Maximum topic length.
pub const MAX_TOPIC_LENGTH: usize = 500;

/// Maximum nickname length.
pub const MAX_NICKNAME_LENGTH: usize = 32;

/// Validate a nickname. Must be 1-32 chars, alphanumeric + underscore/hyphen.
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
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Nickname can only contain letters, numbers, underscores, and hyphens".into());
    }
    Ok(())
}

/// Validate a channel name. Must start with #, 2-50 chars, no spaces.
pub fn validate_channel_name(name: &str) -> Result<(), String> {
    if name.len() < 2 {
        return Err("Channel name too short".into());
    }
    if name.len() > MAX_CHANNEL_NAME_LENGTH {
        return Err(format!(
            "Channel name too long (max {} characters)",
            MAX_CHANNEL_NAME_LENGTH
        ));
    }
    if name.contains(' ') {
        return Err("Channel name cannot contain spaces".into());
    }
    Ok(())
}

/// Validate message content. Must be non-empty and under the length limit.
pub fn validate_message(content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("Message cannot be empty".into());
    }
    if content.len() > MAX_MESSAGE_LENGTH {
        return Err(format!(
            "Message too long (max {} characters)",
            MAX_MESSAGE_LENGTH
        ));
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_nicknames() {
        assert!(validate_nickname("alice").is_ok());
        assert!(validate_nickname("bob_123").is_ok());
        assert!(validate_nickname("user-name").is_ok());
    }

    #[test]
    fn test_invalid_nicknames() {
        assert!(validate_nickname("").is_err());
        assert!(validate_nickname("has space").is_err());
        assert!(validate_nickname("has!special").is_err());
        assert!(validate_nickname(&"a".repeat(33)).is_err());
    }

    #[test]
    fn test_valid_channel_names() {
        assert!(validate_channel_name("#general").is_ok());
        assert!(validate_channel_name("#a").is_ok());
    }

    #[test]
    fn test_invalid_channel_names() {
        assert!(validate_channel_name("#").is_err()); // too short (1 char)
        assert!(validate_channel_name("#has space").is_err());
        assert!(validate_channel_name(&format!("#{}", "a".repeat(50))).is_err());
    }

    #[test]
    fn test_message_validation() {
        assert!(validate_message("hello").is_ok());
        assert!(validate_message("").is_err());
        assert!(validate_message("   ").is_err());
        assert!(validate_message(&"a".repeat(2001)).is_err());
    }

    #[test]
    fn test_topic_validation() {
        assert!(validate_topic("").is_ok()); // empty is ok (clears topic)
        assert!(validate_topic("Welcome!").is_ok());
        assert!(validate_topic(&"a".repeat(501)).is_err());
    }
}
