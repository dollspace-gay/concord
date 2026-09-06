use super::*;

#[test]
fn test_user_profile_serialize_full() {
    let profile = UserProfile {
        id: "user-1".into(),
        username: "alice".into(),
        email: Some("alice@example.com".into()),
        avatar_url: Some("https://example.com/avatar.jpg".into()),
    };
    let json = serde_json::to_value(&profile).unwrap();
    assert_eq!(json["id"], "user-1");
    assert_eq!(json["username"], "alice");
    assert_eq!(json["email"], "alice@example.com");
    assert_eq!(json["avatar_url"], "https://example.com/avatar.jpg");
}

#[test]
fn test_user_profile_serialize_minimal() {
    let profile = UserProfile {
        id: "u1".into(),
        username: "bob".into(),
        email: None,
        avatar_url: None,
    };
    let json = serde_json::to_value(&profile).unwrap();
    assert_eq!(json["id"], "u1");
    assert_eq!(json["username"], "bob");
    assert!(json["email"].is_null());
    assert!(json["avatar_url"].is_null());
}

#[test]
fn test_public_user_profile_serialize() {
    let profile = PublicUserProfile {
        username: "alice".into(),
        avatar_url: Some("https://example.com/pic.jpg".into()),
        provider: Some("github".into()),
        provider_id: Some("12345".into()),
    };
    let json = serde_json::to_value(&profile).unwrap();
    assert_eq!(json["username"], "alice");
    assert_eq!(json["provider"], "github");
    assert_eq!(json["provider_id"], "12345");
}

#[test]
fn test_public_user_profile_serialize_no_optionals() {
    let profile = PublicUserProfile {
        username: "bob".into(),
        avatar_url: None,
        provider: None,
        provider_id: None,
    };
    let json = serde_json::to_value(&profile).unwrap();
    assert_eq!(json["username"], "bob");
    assert!(json["avatar_url"].is_null());
    assert!(json["provider"].is_null());
}

#[test]
fn test_update_profile_request_full() {
    let json = r#"{"bio": "Hello!", "pronouns": "they/them", "banner_url": "https://example.com/banner.jpg"}"#;
    let req: UpdateProfileRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.bio, Some("Hello!".into()));
    assert_eq!(req.pronouns, Some("they/them".into()));
    assert_eq!(
        req.banner_url,
        Some("https://example.com/banner.jpg".into())
    );
}

#[test]
fn test_update_profile_request_empty() {
    let json = r#"{}"#;
    let req: UpdateProfileRequest = serde_json::from_str(json).unwrap();
    assert!(req.bio.is_none());
    assert!(req.pronouns.is_none());
    assert!(req.banner_url.is_none());
}
