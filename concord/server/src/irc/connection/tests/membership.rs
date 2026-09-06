use super::*;

#[test]
fn test_join_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Join {
            nickname: "alice".into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            avatar_url: None,
            user_id: Some("alice-id".into()),
            server_avatar_url: None,
            role_ids: Vec::new(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("JOIN #general"));
    assert!(lines[0].starts_with(":alice!"));
}

#[test]
fn test_server_notice_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ServerNotice {
            message: "Welcome to Concord".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("NOTICE viewer :Welcome to Concord"));
}

#[test]
fn test_member_kick_event_with_reason() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MemberKick {
            server_id: DEFAULT_SERVER_ID.into(),
            user_id: "uid1".into(),
            kicked_by: "admin".into(),
            reason: Some("Rule violation".into()),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("admin"));
    assert!(lines[0].contains("kicked"));
    assert!(lines[0].contains("Rule violation"));
}

#[test]
fn test_member_kick_event_no_reason() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MemberKick {
            server_id: DEFAULT_SERVER_ID.into(),
            user_id: "uid1".into(),
            kicked_by: "admin".into(),
            reason: None,
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("No reason given"));
}

#[test]
fn test_member_ban_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MemberBan {
            server_id: DEFAULT_SERVER_ID.into(),
            user_id: "uid1".into(),
            banned_by: "admin".into(),
            reason: Some("Spam".into()),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("banned"));
    assert!(lines[0].contains("Spam"));
}

#[test]
fn test_member_unban_is_silent() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MemberUnban {
            server_id: DEFAULT_SERVER_ID.into(),
            user_id: "uid1".into(),
        },
    );
    assert!(lines.is_empty());
}
