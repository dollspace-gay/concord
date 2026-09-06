use super::*;

#[test]
fn test_create_server_request_full() {
    let json = r#"{"name": "My Server", "icon_url": "https://example.com/icon.png"}"#;
    let req: CreateServerRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "My Server");
    assert_eq!(req.icon_url, Some("https://example.com/icon.png".into()));
}

#[test]
fn test_create_server_request_name_only() {
    let json = r#"{"name": "Test"}"#;
    let req: CreateServerRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "Test");
    assert!(req.icon_url.is_none());
}

#[test]
fn test_create_token_request_with_label() {
    let json = r#"{"label": "My IRC client"}"#;
    let req: CreateTokenRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.label, Some("My IRC client".into()));
}

#[test]
fn test_create_token_request_no_label() {
    let json = r#"{}"#;
    let req: CreateTokenRequest = serde_json::from_str(json).unwrap();
    assert!(req.label.is_none());
}

#[test]
fn test_create_token_response_serialize() {
    let resp = CreateTokenResponse {
        id: "tok-1".into(),
        token: "abcdef123456".into(),
        label: Some("dev".into()),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["id"], "tok-1");
    assert_eq!(json["token"], "abcdef123456");
    assert_eq!(json["label"], "dev");
}

#[test]
fn test_create_emoji_request() {
    let json = r#"{"name": "smile", "image_url": "https://example.com/smile.png"}"#;
    let req: CreateEmojiRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "smile");
    assert_eq!(req.image_url, "https://example.com/smile.png");
}
