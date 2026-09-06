use super::*;

#[test]
fn test_auth_status_response_serialize() {
    let resp = AuthStatusResponse {
        authenticated: false,
        providers: vec!["atproto".into()],
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["authenticated"], false);
    let providers = json["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0], "atproto");
}

#[test]
fn test_irc_token_info_serialize() {
    let info = IrcTokenInfo {
        id: "t1".into(),
        label: Some("test".into()),
        last_used: Some("2025-01-01T00:00:00Z".into()),
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["id"], "t1");
    assert_eq!(json["label"], "test");
    assert_eq!(json["last_used"], "2025-01-01T00:00:00Z");
    assert_eq!(json["created_at"], "2025-01-01T00:00:00Z");
}

#[test]
fn test_irc_token_info_serialize_no_optionals() {
    let info = IrcTokenInfo {
        id: "t2".into(),
        label: None,
        last_used: None,
        created_at: "2025-01-01".into(),
    };
    let json = serde_json::to_value(&info).unwrap();
    assert!(json["label"].is_null());
    assert!(json["last_used"].is_null());
}

#[test]
fn test_upload_response_serialize() {
    let resp = UploadResponse {
        id: "att-1".into(),
        filename: "photo.jpg".into(),
        content_type: "image/jpeg".into(),
        file_size: 1024,
        url: "/api/uploads/att-1".into(),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["id"], "att-1");
    assert_eq!(json["filename"], "photo.jpg");
    assert_eq!(json["content_type"], "image/jpeg");
    assert_eq!(json["file_size"], 1024);
    assert_eq!(json["url"], "/api/uploads/att-1");
}

#[test]
fn test_emoji_response_serialize() {
    let resp = EmojiResponse {
        id: "e1".into(),
        server_id: "s1".into(),
        name: "thumbsup".into(),
        image_url: "/api/uploads/emoji.png".into(),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["name"], "thumbsup");
    assert_eq!(json["server_id"], "s1");
}

#[test]
fn test_history_response_serialize() {
    let resp = HistoryResponse {
        channel: "#general".into(),
        messages: vec![],
        has_more: false,
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["channel"], "#general");
    assert_eq!(json["messages"].as_array().unwrap().len(), 0);
    assert_eq!(json["has_more"], false);
}

#[test]
fn test_history_response_serialize_has_more() {
    let resp = HistoryResponse {
        channel: "#dev".into(),
        messages: vec![],
        has_more: true,
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["has_more"], true);
}

#[test]
fn test_malformed_content_type_rejected() {
    assert!(!is_allowed_upload_content_type("notamimetype"));
    assert!(!is_allowed_upload_content_type(""));
    // Excessively long content type
    let long_type = format!("image/{}", "x".repeat(300));
    assert!(!is_allowed_upload_content_type(&long_type));
}
