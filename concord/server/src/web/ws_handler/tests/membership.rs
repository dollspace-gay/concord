use super::*;

#[test]
fn test_join_channel() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "join_channel",
        "server_id": "srv-1",
        "channel": "#random"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::JoinChannel { server_id, channel } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(channel, "#random");
        }
        _ => panic!("Expected JoinChannel"),
    }
}

#[test]
fn test_part_channel() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "part_channel",
        "server_id": "srv-1",
        "channel": "#random",
        "reason": "Going offline"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::PartChannel {
            server_id,
            channel,
            reason,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(channel, "#random");
            assert_eq!(reason, Some("Going offline".into()));
        }
        _ => panic!("Expected PartChannel"),
    }
}

#[test]
fn test_join_server() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "join_server",
        "server_id": "srv-1"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::JoinServer { server_id } if server_id == "srv-1"));
}

#[test]
fn test_leave_server() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "leave_server",
        "server_id": "srv-1"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::LeaveServer { server_id } if server_id == "srv-1"));
}

#[test]
fn test_kick_member() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "kick_member",
        "server_id": "srv-1",
        "user_id": "user-1",
        "reason": "Spamming"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::KickMember {
            server_id,
            user_id,
            reason,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(user_id, "user-1");
            assert_eq!(reason, Some("Spamming".into()));
        }
        _ => panic!("Expected KickMember"),
    }
}

#[test]
fn test_ban_member() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "ban_member",
        "server_id": "srv-1",
        "user_id": "user-1",
        "reason": "Harassment",
        "delete_message_days": 7
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::BanMember {
            server_id,
            user_id,
            reason,
            delete_message_days,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(user_id, "user-1");
            assert_eq!(reason, Some("Harassment".into()));
            assert_eq!(delete_message_days, 7);
        }
        _ => panic!("Expected BanMember"),
    }
}

#[test]
fn test_discover_servers() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "discover_servers",
        "category": "gaming"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::DiscoverServers { category } => {
            assert_eq!(category, Some("gaming".into()));
        }
        _ => panic!("Expected DiscoverServers"),
    }
}

#[test]
fn test_follow_channel() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "follow_channel",
        "source_channel_id": "ch-1",
        "target_channel_id": "ch-2"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::FollowChannel {
            source_channel_id,
            target_channel_id,
        } => {
            assert_eq!(source_channel_id, "ch-1");
            assert_eq!(target_channel_id, "ch-2");
        }
        _ => panic!("Expected FollowChannel"),
    }
}
