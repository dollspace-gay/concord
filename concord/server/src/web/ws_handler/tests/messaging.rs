use super::*;

#[test]
fn test_send_message_basic() {
    let msg: ClientMessage = parse_msg(
        r##"{"type": "send_message", "operation_generation": "generation-0001", "channel": "#general", "content": "Hello world"}"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SendMessage {
            server_id,
            channel,
            content,
            reply_to,
            attachment_ids,
            nonce,
            ..
        } => {
            assert_eq!(server_id, DEFAULT_SERVER_ID);
            assert_eq!(channel, "#general");
            assert_eq!(content, "Hello world");
            assert!(reply_to.is_none());
            assert!(attachment_ids.is_none());
            assert!(nonce.is_none());
        }
        _ => panic!("Expected SendMessage"),
    }
}

#[test]
fn test_send_message_with_reply_and_attachments() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "send_message",
        "operation_generation": "generation-0001",
        "server_id": "srv-1",
        "channel": "#dev",
        "content": "See attached",
        "reply_to": "msg-123",
        "attachment_ids": ["att-1", "att-2"]
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SendMessage {
            server_id,
            reply_to,
            attachment_ids,
            ..
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(reply_to, Some("msg-123".into()));
            assert_eq!(attachment_ids, Some(vec!["att-1".into(), "att-2".into()]));
        }
        _ => panic!("Expected SendMessage"),
    }
}

#[test]
fn test_edit_message() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "edit_message",
        "operation_generation": "generation-0001",
        "message_id": "msg-1",
        "content": "edited content"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::EditMessage {
            message_id,
            content,
            ..
        } => {
            assert_eq!(message_id, "msg-1");
            assert_eq!(content, "edited content");
        }
        _ => panic!("Expected EditMessage"),
    }
}

#[test]
fn test_delete_message() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "delete_message",
        "operation_generation": "generation-0001",
        "message_id": "msg-1"
    }"##,
    )
    .unwrap();
    assert!(
        matches!(msg, ClientMessage::DeleteMessage { message_id, .. } if message_id == "msg-1")
    );
}

#[test]
fn test_add_reaction() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "add_reaction",
        "operation_generation": "generation-0001",
        "message_id": "msg-1",
        "emoji": "\ud83d\udc4d"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::AddReaction {
            message_id, emoji, ..
        } => {
            assert_eq!(message_id, "msg-1");
            assert_eq!(emoji, "\u{1f44d}");
        }
        _ => panic!("Expected AddReaction"),
    }
}

#[test]
fn test_remove_reaction() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "remove_reaction",
        "operation_generation": "generation-0001",
        "message_id": "msg-1",
        "emoji": "\ud83d\udc4d"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::RemoveReaction { .. }));
}

#[test]
fn test_invoke_message_component() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "invoke_message_component",
        "request_id": "request-2",
        "message_id": "message-1",
        "custom_id": "priority",
        "values": ["high"]
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::InvokeMessageComponent {
            request_id,
            message_id,
            custom_id,
            values,
        } => {
            assert_eq!(request_id, "request-2");
            assert_eq!(message_id, "message-1");
            assert_eq!(custom_id, "priority");
            assert_eq!(values, ["high"]);
        }
        _ => panic!("Expected InvokeMessageComponent"),
    }
}

#[test]
fn test_pin_message() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "pin_message",
        "server_id": "srv-1",
        "channel": "#general",
        "message_id": "msg-1"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::PinMessage {
            server_id,
            channel,
            message_id,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(channel, "#general");
            assert_eq!(message_id, "msg-1");
        }
        _ => panic!("Expected PinMessage"),
    }
}

#[test]
fn test_unpin_message() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "unpin_message",
        "server_id": "srv-1",
        "channel": "#general",
        "message_id": "msg-1"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::UnpinMessage { .. }));
}

#[test]
fn test_search_messages() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "search_messages",
        "server_id": "srv-1",
        "query": "hello world",
        "channel": "#general",
        "limit": 10,
        "offset": 5
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SearchMessages {
            query,
            channel,
            limit,
            offset,
            ..
        } => {
            assert_eq!(query, "hello world");
            assert_eq!(channel, Some("#general".into()));
            assert_eq!(limit, Some(10));
            assert_eq!(offset, Some(5));
        }
        _ => panic!("Expected SearchMessages"),
    }
}
