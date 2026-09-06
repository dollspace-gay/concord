use super::*;

#[test]
fn test_message_event_roundtrip() {
    let event = ChatEvent::Message {
        id: Uuid::new_v4().into(),
        server_id: Some("srv1".into()),
        conversation_id: None,
        from: "alice".into(),
        target: "#general".into(),
        content: "Hello, world!".into(),
        timestamp: Utc::now(),
        avatar_url: Some("https://example.com/avatar.png".into()),
        reply_to: Some(ReplyInfo {
            id: "msg-123".into(),
            from: "bob".into(),
            content_preview: "earlier message".into(),
        }),
        attachments: Some(vec![AttachmentInfo {
            id: "att-1".into(),
            filename: "file.txt".into(),
            content_type: "text/plain".into(),
            file_size: 1234,
            url: "https://example.com/file.txt".into(),
        }]),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::Message {
            from,
            target,
            content,
            server_id,
            reply_to,
            attachments,
            ..
        } => {
            assert_eq!(from, "alice");
            assert_eq!(target, "#general");
            assert_eq!(content, "Hello, world!");
            assert_eq!(server_id, Some("srv1".into()));
            assert!(reply_to.is_some());
            assert_eq!(reply_to.unwrap().from, "bob");
            assert_eq!(attachments.unwrap().len(), 1);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_message_event_minimal() {
    let event = ChatEvent::Message {
        id: Uuid::new_v4().into(),
        server_id: None,
        conversation_id: None,
        from: "alice".into(),
        target: "bob".into(),
        content: "DM".into(),
        timestamp: Utc::now(),
        avatar_url: None,
        reply_to: None,
        attachments: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    // Optional None fields should be skipped
    assert!(!json.contains("server_id"));
    assert!(!json.contains("avatar_url"));
    assert!(!json.contains("reply_to"));
    assert!(!json.contains("attachments"));
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::Message { from, target, .. } => {
            assert_eq!(from, "alice");
            assert_eq!(target, "bob");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_message_edit_event_roundtrip() {
    let event = ChatEvent::MessageEdit {
        id: Uuid::new_v4().into(),
        server_id: "srv1".into(),
        channel: "#general".into(),
        content: "edited content".into(),
        edited_at: Utc::now(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::MessageEdit {
            content, channel, ..
        } => {
            assert_eq!(content, "edited content");
            assert_eq!(channel, "#general");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_message_delete_event_roundtrip() {
    let event = ChatEvent::MessageDelete {
        id: Uuid::new_v4().into(),
        server_id: "srv1".into(),
        channel: "#general".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::MessageDelete { channel, .. } => {
            assert_eq!(channel, "#general");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_reaction_add_event_roundtrip() {
    let event = ChatEvent::ReactionAdd {
        message_id: Uuid::new_v4().into(),
        server_id: "srv1".into(),
        channel: "#general".into(),
        user_id: "user1".into(),
        nickname: "alice".into(),
        emoji: "\u{1F44D}".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::ReactionAdd {
            emoji, nickname, ..
        } => {
            assert_eq!(emoji, "\u{1F44D}");
            assert_eq!(nickname, "alice");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_bulk_message_delete_event_roundtrip() {
    let event = ChatEvent::BulkMessageDelete {
        server_id: "srv1".into(),
        channel: "#general".into(),
        message_ids: vec!["msg1".into(), "msg2".into(), "msg3".into()],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::BulkMessageDelete { message_ids, .. } => {
            assert_eq!(message_ids.len(), 3);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_pinned_messages_event_roundtrip() {
    let event = ChatEvent::PinnedMessages {
        server_id: "srv1".into(),
        channel: "#general".into(),
        pins: vec![PinnedMessageInfo {
            id: "pin1".into(),
            message_id: "msg1".into(),
            channel_id: "ch1".into(),
            pinned_by: "user1".into(),
            pinned_at: "2026-01-01T00:00:00Z".into(),
            from: "alice".into(),
            content: "Important message".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
        }],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::PinnedMessages { pins, .. } => {
            assert_eq!(pins.len(), 1);
            assert_eq!(pins[0].content, "Important message");
        }
        _ => panic!("Wrong variant"),
    }
}
