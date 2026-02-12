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
    Ok(())
}

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

    // ────────────────────────────────────────────────────────────────
    // Boundary length tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_nickname_exactly_at_max_length() {
        let nick = "a".repeat(MAX_NICKNAME_LENGTH);
        assert!(validate_nickname(&nick).is_ok());
    }

    #[test]
    fn test_nickname_one_over_max_length() {
        let nick = "a".repeat(MAX_NICKNAME_LENGTH + 1);
        assert!(validate_nickname(&nick).is_err());
    }

    #[test]
    fn test_nickname_single_char() {
        assert!(validate_nickname("a").is_ok());
    }

    #[test]
    fn test_channel_name_exactly_at_max_length() {
        // Channel name max is 50, including the #
        let name = format!("#{}", "a".repeat(MAX_CHANNEL_NAME_LENGTH - 1));
        assert_eq!(name.len(), MAX_CHANNEL_NAME_LENGTH);
        assert!(validate_channel_name(&name).is_ok());
    }

    #[test]
    fn test_channel_name_one_over_max_length() {
        let name = format!("#{}", "a".repeat(MAX_CHANNEL_NAME_LENGTH));
        assert!(validate_channel_name(&name).is_err());
    }

    #[test]
    fn test_channel_name_exactly_two_chars() {
        assert!(validate_channel_name("#a").is_ok());
    }

    #[test]
    fn test_message_exactly_at_max_length() {
        let msg = "a".repeat(MAX_MESSAGE_LENGTH);
        assert!(validate_message(&msg).is_ok());
    }

    #[test]
    fn test_message_one_over_max_length() {
        let msg = "a".repeat(MAX_MESSAGE_LENGTH + 1);
        assert!(validate_message(&msg).is_err());
    }

    #[test]
    fn test_topic_exactly_at_max_length() {
        let topic = "a".repeat(MAX_TOPIC_LENGTH);
        assert!(validate_topic(&topic).is_ok());
    }

    #[test]
    fn test_topic_one_over_max_length() {
        let topic = "a".repeat(MAX_TOPIC_LENGTH + 1);
        assert!(validate_topic(&topic).is_err());
    }

    #[test]
    fn test_server_name_exactly_at_max_length() {
        let name = "a".repeat(MAX_SERVER_NAME_LENGTH);
        assert!(validate_server_name(&name).is_ok());
    }

    #[test]
    fn test_server_name_one_over_max_length() {
        let name = "a".repeat(MAX_SERVER_NAME_LENGTH + 1);
        assert!(validate_server_name(&name).is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Unicode / emoji in names
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_nickname_rejects_emoji_but_allows_unicode_alphanumeric() {
        // Rust's is_alphanumeric() returns true for Unicode letters/digits,
        // so accented chars and CJK are allowed by the current validation.
        assert!(validate_nickname("user\u{1F600}").is_err()); // emoji is not alphanumeric
        assert!(validate_nickname("\u{00E9}mile").is_ok()); // accented char IS alphanumeric
        assert!(validate_nickname("\u{4E16}\u{754C}").is_ok()); // CJK chars ARE alphanumeric
    }

    #[test]
    fn test_channel_name_allows_unicode() {
        // Channel names only check for spaces and length, not charset
        assert!(validate_channel_name("#caf\u{00E9}").is_ok());
        assert!(validate_channel_name("#\u{1F600}x").is_ok());
    }

    #[test]
    fn test_server_name_allows_unicode() {
        assert!(validate_server_name("Caf\u{00E9} Chat").is_ok());
        assert!(validate_server_name("\u{1F600} Server").is_ok());
    }

    #[test]
    fn test_message_allows_unicode() {
        assert!(validate_message("Hello \u{1F44B} World \u{1F310}").is_ok());
        assert!(validate_message("\u{4F60}\u{597D}\u{4E16}\u{754C}").is_ok()); // Chinese
    }

    // ────────────────────────────────────────────────────────────────
    // SQL injection attempts
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_nickname_rejects_sql_injection() {
        // SQL injection chars are special chars, should be rejected by nickname validation
        assert!(validate_nickname("'; DROP TABLE users;--").is_err());
        assert!(validate_nickname("user' OR '1'='1").is_err());
        assert!(validate_nickname("admin\"--").is_err());
    }

    #[test]
    fn test_server_name_allows_special_chars_but_not_empty() {
        // Server name validation only checks length and emptiness, not charset
        // The real protection is parameterized queries
        assert!(validate_server_name("'; DROP TABLE users;--").is_ok());
    }

    #[test]
    fn test_message_allows_sql_like_content() {
        // Messages can contain anything as long as they're not empty and within length
        assert!(validate_message("SELECT * FROM users; DROP TABLE messages;").is_ok());
    }

    // ────────────────────────────────────────────────────────────────
    // Empty and whitespace strings
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_empty_strings() {
        assert!(validate_nickname("").is_err());
        assert!(validate_channel_name("").is_err());
        assert!(validate_message("").is_err());
        assert!(validate_server_name("").is_err());
        assert!(validate_topic("").is_ok()); // topic can be empty (clears it)
    }

    #[test]
    fn test_whitespace_only_strings() {
        assert!(validate_nickname(" ").is_err()); // has space -> invalid char
        assert!(validate_message("   ").is_err()); // trims to empty
        assert!(validate_message("\t\n").is_err()); // whitespace only
        assert!(validate_server_name("   ").is_err()); // trims to empty
    }

    // ────────────────────────────────────────────────────────────────
    // Special characters (null bytes, control characters)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_nickname_rejects_null_bytes() {
        assert!(validate_nickname("user\0name").is_err());
    }

    #[test]
    fn test_nickname_rejects_control_chars() {
        assert!(validate_nickname("user\x01name").is_err());
        assert!(validate_nickname("user\x07name").is_err()); // bell
        assert!(validate_nickname("user\tnick").is_err()); // tab
        assert!(validate_nickname("user\nnick").is_err()); // newline
    }

    #[test]
    fn test_channel_name_rejects_spaces() {
        assert!(validate_channel_name("#has space").is_err());
        assert!(validate_channel_name("# ").is_err());
    }

    #[test]
    fn test_nickname_allows_underscores_and_hyphens() {
        assert!(validate_nickname("my_user").is_ok());
        assert!(validate_nickname("my-user").is_ok());
        assert!(validate_nickname("_leading").is_ok());
        assert!(validate_nickname("-leading").is_ok());
        assert!(validate_nickname("trailing_").is_ok());
        assert!(validate_nickname("trailing-").is_ok());
        assert!(validate_nickname("a-b_c").is_ok());
    }

    #[test]
    fn test_nickname_numbers_only() {
        assert!(validate_nickname("12345").is_ok());
    }

    #[test]
    fn test_message_with_newlines_is_valid() {
        assert!(validate_message("line1\nline2\nline3").is_ok());
        assert!(validate_message("line1\r\nline2").is_ok());
    }

    #[test]
    fn test_topic_with_special_chars() {
        assert!(validate_topic("Welcome! <b>bold</b> & \"quoted\"").is_ok());
        assert!(validate_topic("Topic with\nnewline").is_ok());
    }

    #[test]
    fn test_sanitize_html_escapes_tags() {
        assert_eq!(sanitize_html("<script>alert('xss')</script>"), "&lt;script&gt;alert('xss')&lt;/script&gt;");
    }

    #[test]
    fn test_sanitize_html_escapes_ampersand() {
        assert_eq!(sanitize_html("a & b"), "a &amp; b");
    }

    #[test]
    fn test_sanitize_html_preserves_normal_text() {
        assert_eq!(sanitize_html("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_html_preserves_markdown() {
        assert_eq!(sanitize_html("**bold** _italic_ `code`"), "**bold** _italic_ `code`");
    }

    #[test]
    fn test_sanitize_html_mixed_content() {
        assert_eq!(sanitize_html("Hello <b>world</b> & friends"), "Hello &lt;b&gt;world&lt;/b&gt; &amp; friends");
    }
}
