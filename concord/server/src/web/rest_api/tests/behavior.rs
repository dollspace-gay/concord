use super::*;

#[test]
fn test_history_params_full() {
    let json = r#"{"server_id": "srv-1", "before": "msg-abc", "limit": 100}"#;
    let params: HistoryParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.server_id, Some("srv-1".into()));
    assert_eq!(params.before, Some("msg-abc".into()));
    assert_eq!(params.limit, Some(100));
}

#[test]
fn test_history_params_minimal() {
    let json = r#"{}"#;
    let params: HistoryParams = serde_json::from_str(json).unwrap();
    assert!(params.server_id.is_none());
    assert!(params.before.is_none());
    assert!(params.limit.is_none());
}

#[test]
fn test_set_admin_request_true() {
    let json = r#"{"is_admin": true}"#;
    let req: SetAdminRequest = serde_json::from_str(json).unwrap();
    assert!(req.is_admin);
}

#[test]
fn test_set_admin_request_false() {
    let json = r#"{"is_admin": false}"#;
    let req: SetAdminRequest = serde_json::from_str(json).unwrap();
    assert!(!req.is_admin);
}

#[test]
fn test_discover_params_with_category() {
    let json = r#"{"category": "gaming"}"#;
    let params: DiscoverParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.category, Some("gaming".into()));
}

#[test]
fn test_discover_params_empty() {
    let json = r#"{}"#;
    let params: DiscoverParams = serde_json::from_str(json).unwrap();
    assert!(params.category.is_none());
}

#[test]
fn test_webhook_execute_request_full() {
    let json = r#"{"content": "Hello from webhook", "idempotency_key": "request-1", "username": "Bot", "avatar_url": "https://example.com/bot.png"}"#;
    let req: WebhookExecuteRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.content, "Hello from webhook");
    assert_eq!(req.idempotency_key, "request-1");
    assert_eq!(req.username, Some("Bot".into()));
    assert_eq!(req.avatar_url, Some("https://example.com/bot.png".into()));
}

#[test]
fn test_webhook_execute_request_content_only() {
    let json = r#"{"content": "test message", "idempotency_key": "request-2"}"#;
    let req: WebhookExecuteRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.content, "test message");
    assert!(req.username.is_none());
    assert!(req.avatar_url.is_none());
}

#[test]
fn test_allowed_upload_content_types() {
    assert!(is_allowed_upload_content_type("image/jpeg"));
    assert!(is_allowed_upload_content_type("image/png"));
    assert!(is_allowed_upload_content_type("image/gif"));
    assert!(is_allowed_upload_content_type("image/webp"));
    assert!(is_allowed_upload_content_type("video/mp4"));
    assert!(is_allowed_upload_content_type("audio/mpeg"));
    assert!(is_allowed_upload_content_type("application/pdf"));
    assert!(is_allowed_upload_content_type("application/octet-stream"));
    assert!(is_allowed_upload_content_type("text/plain"));
    assert!(is_allowed_upload_content_type("text/css"));
}

#[test]
fn test_blocked_upload_content_types() {
    assert!(!is_allowed_upload_content_type("text/html"));
    assert!(!is_allowed_upload_content_type("text/javascript"));
    assert!(!is_allowed_upload_content_type("application/javascript"));
    assert!(!is_allowed_upload_content_type("application/xhtml+xml"));
    assert!(!is_allowed_upload_content_type("image/svg+xml"));
    assert!(!is_allowed_upload_content_type("text/xml"));
    assert!(!is_allowed_upload_content_type("application/xml"));
}

#[test]
fn active_media_types_are_always_downloads() {
    assert!(safe_inline_content_type("image/png"));
    assert!(!safe_inline_content_type("image/svg+xml"));
    assert!(!safe_inline_content_type("text/html"));
    assert!(!safe_inline_content_type("application/pdf"));
}

#[test]
fn test_blocked_content_type_with_params() {
    // Should still block even with charset parameters
    assert!(!is_allowed_upload_content_type("text/html; charset=utf-8"));
    assert!(!is_allowed_upload_content_type(
        "application/javascript; charset=utf-8"
    ));
}

#[test]
fn test_blocked_content_type_case_insensitive() {
    assert!(!is_allowed_upload_content_type("Text/HTML"));
    assert!(!is_allowed_upload_content_type("APPLICATION/JAVASCRIPT"));
    assert!(!is_allowed_upload_content_type("Image/SVG+XML"));
}
