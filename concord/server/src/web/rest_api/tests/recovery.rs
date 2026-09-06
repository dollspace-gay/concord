use super::*;

#[test]
fn test_create_server_request_missing_name_fails() {
    let json = r#"{"icon_url": "https://example.com/icon.png"}"#;
    assert!(serde_json::from_str::<CreateServerRequest>(json).is_err());
}

#[test]
fn test_set_admin_request_missing_field_fails() {
    let json = r#"{}"#;
    assert!(serde_json::from_str::<SetAdminRequest>(json).is_err());
}

#[test]
fn test_create_emoji_request_missing_name_fails() {
    let json = r#"{"image_url": "url"}"#;
    assert!(serde_json::from_str::<CreateEmojiRequest>(json).is_err());
}

#[test]
fn test_create_emoji_request_missing_url_fails() {
    let json = r#"{"name": "smile"}"#;
    assert!(serde_json::from_str::<CreateEmojiRequest>(json).is_err());
}

#[test]
fn test_search_params_missing_required_fails() {
    let json = r#"{"q": "test"}"#;
    assert!(serde_json::from_str::<SearchParams>(json).is_err());
}

#[test]
fn test_webhook_execute_request_missing_content_fails() {
    let json = r#"{"idempotency_key": "request-3", "username": "Bot"}"#;
    assert!(serde_json::from_str::<WebhookExecuteRequest>(json).is_err());
}

#[test]
fn test_webhook_execute_request_missing_idempotency_key_fails() {
    let json = r#"{"content": "test message"}"#;
    assert!(serde_json::from_str::<WebhookExecuteRequest>(json).is_err());
}
