use super::*;

#[test]
fn ircv3_message_tags_preserve_historical_opaque_ids() {
    let caps = ClientCaps {
        server_time: true,
        message_tags: true,
        sasl: false,
    };
    let timestamp = chrono::DateTime::parse_from_rfc3339("2024-01-02T03:04:06.654321-05:00")
        .unwrap()
        .with_timezone(&Utc);
    let id = MessageId::from_stored("  legacy;message\\id  ").unwrap();
    assert_eq!(
        build_history_tag_prefix(&caps, &id, &timestamp),
        "@time=2024-01-02T08:04:06.654321+00:00;msgid=\\s\\slegacy\\:message\\\\id\\s\\s "
    );
}

#[test]
fn test_message_event_to_channel() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Message {
            id: Uuid::new_v4().into(),
            server_id: Some(DEFAULT_SERVER_ID.to_string()),
            conversation_id: None,
            from: "alice".into(),
            target: "#general".into(),
            content: "Hello world".into(),
            timestamp: Utc::now(),
            avatar_url: None,
            reply_to: None,
            attachments: None,
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("PRIVMSG #general :Hello world"));
    assert!(lines[0].starts_with(":alice!"));
}

#[test]
fn test_message_event_dm() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "bob",
        &ChatEvent::Message {
            id: Uuid::new_v4().into(),
            server_id: None,
            conversation_id: None,
            from: "alice".into(),
            target: "bob".into(),
            content: "Hey there".into(),
            timestamp: Utc::now(),
            avatar_url: None,
            reply_to: None,
            attachments: None,
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("PRIVMSG bob :Hey there"));
}

#[test]
fn test_message_edit_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MessageEdit {
            id: Uuid::new_v4().into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            content: "edited content".into(),
            edited_at: Utc::now(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("NOTICE viewer"));
    assert!(lines[0].contains("edited"));
    assert!(lines[0].contains("#general"));
}

#[test]
fn test_message_delete_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MessageDelete {
            id: Uuid::new_v4().into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("NOTICE viewer"));
    assert!(lines[0].contains("deleted"));
    assert!(lines[0].contains("#general"));
}

#[test]
fn test_reaction_add_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ReactionAdd {
            message_id: Uuid::new_v4().into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            user_id: "uid1".into(),
            nickname: "alice".into(),
            emoji: "\u{1f44d}".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("alice"));
    assert!(lines[0].contains("\u{1f44d}"));
    assert!(lines[0].contains("#general"));
}

#[test]
fn test_reaction_remove_event_formats_action() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ReactionRemove {
            message_id: Uuid::new_v4().into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            user_id: "uid1".into(),
            nickname: "alice".into(),
            emoji: "\u{1f44d}".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("ACTION"));
    assert!(lines[0].contains("removed reaction"));
}

#[test]
fn test_message_embed_is_silent() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MessageEmbed {
            message_id: Uuid::new_v4().into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            embeds: vec![],
        },
    );
    assert!(lines.is_empty());
}

#[test]
fn test_message_pin_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MessagePin {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            pin: PinnedMessageInfo {
                id: "pin-1".into(),
                message_id: "msg-1".into(),
                channel_id: "ch-1".into(),
                pinned_by: "alice".into(),
                pinned_at: "2025-01-01".into(),
                from: "bob".into(),
                content: "Important msg".into(),
                timestamp: "2025-01-01".into(),
            },
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("alice"));
    assert!(lines[0].contains("pinned"));
    assert!(lines[0].contains("bob"));
}

#[test]
fn test_message_unpin_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::MessageUnpin {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            message_id: "msg-1".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("unpinned"));
}

#[test]
fn test_bulk_message_delete_is_silent() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::BulkMessageDelete {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            message_ids: vec!["msg-1".into(), "msg-2".into()],
        },
    );
    assert!(lines.is_empty());
}

#[test]
fn test_send_line_sends_to_channel() {
    let (tx, mut rx) = mpsc::channel::<OutboundLine>(1024);
    let out = Outbound {
        tx,
        failed: CancellationToken::new(),
        actor: Arc::new(std::sync::RwLock::new(None)),
        queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    send_line(&out, "PRIVMSG #test :Hello");
    let received = rx.try_recv().unwrap();
    assert_eq!(received.line, "PRIVMSG #test :Hello");
    assert!(received.guard.is_none());
}

#[test]
fn test_send_line_closed_channel_does_not_panic() {
    let (tx, rx) = mpsc::channel::<OutboundLine>(1024);
    drop(rx); // Close the receiver
    let failed = CancellationToken::new();
    let out = Outbound {
        tx,
        failed: failed.clone(),
        actor: Arc::new(std::sync::RwLock::new(None)),
        queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    send_line(&out, "PRIVMSG #test :Hello");
    assert!(failed.is_cancelled());
}

#[test]
fn test_send_line_full_channel_marks_transport_failed() {
    let (tx, _rx) = mpsc::channel::<OutboundLine>(1);
    let failed = CancellationToken::new();
    let out = Outbound {
        tx,
        failed: failed.clone(),
        actor: Arc::new(std::sync::RwLock::new(None)),
        queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    send_line(&out, "first");
    send_line(&out, "overflow");
    assert!(failed.is_cancelled());
}
