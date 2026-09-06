use super::*;

#[test]
fn test_fetch_history() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "fetch_history",
        "server_id": "srv-1",
        "channel": "#general",
        "before": "2025-01-01T00:00:00Z",
        "limit": 25
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::FetchHistory { before, limit, .. } => {
            assert_eq!(before, Some("2025-01-01T00:00:00Z".into()));
            assert_eq!(limit, Some(25));
        }
        _ => panic!("Expected FetchHistory"),
    }
}

#[test]
fn test_list_channels() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "list_channels",
        "server_id": "srv-1"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::ListChannels { server_id } => {
            assert_eq!(server_id, "srv-1");
        }
        _ => panic!("Expected ListChannels"),
    }
}

#[test]
fn test_list_servers() {
    let msg: ClientMessage = parse_msg(r##"{"type": "list_servers"}"##).unwrap();
    assert!(matches!(msg, ClientMessage::ListServers));
}

#[test]
fn test_list_roles() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "list_roles",
        "server_id": "srv-1"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::ListRoles { server_id } if server_id == "srv-1"));
}

#[test]
fn test_list_bookmarks() {
    let msg: ClientMessage = parse_msg(r##"{"type": "list_bookmarks"}"##).unwrap();
    assert!(matches!(msg, ClientMessage::ListBookmarks));
}

#[test]
fn test_list_oauth2_apps() {
    let msg: ClientMessage = parse_msg(r##"{"type": "list_o_auth2_apps"}"##).unwrap();
    assert!(matches!(msg, ClientMessage::ListOAuth2Apps));
}

#[test]
fn test_get_audit_log() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "get_audit_log",
        "server_id": "srv-1",
        "action_type": "ban",
        "limit": 25
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::GetAuditLog {
            server_id,
            action_type,
            limit,
            before,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(action_type, Some("ban".into()));
            assert_eq!(limit, Some(25));
            assert!(before.is_none());
        }
        _ => panic!("Expected GetAuditLog"),
    }
}
