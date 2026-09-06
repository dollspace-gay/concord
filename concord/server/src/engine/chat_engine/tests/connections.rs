use super::*;

#[test]
fn irc_alias_is_stable_bounded_and_not_display_name_routing() {
    assert_eq!(
        stable_irc_alias("My Long Server Name!", "12345678-rest"),
        "my-long-server-name-12345678"
    );
    assert_eq!(stable_irc_alias("🦇", "abcdef"), "server-abcdef");
    assert_eq!(
        stable_irc_alias("Concord", "🦇archive"),
        "concord-🦇archive"
    );
}

#[tokio::test]
async fn test_connect_and_disconnect() {
    let engine = setup_engine();

    let (session_id, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    assert!(!engine.is_nick_available("alice"));

    engine.disconnect(session_id);
    assert!(engine.is_nick_available("alice"));
}

#[tokio::test]
async fn test_same_user_can_hold_multiple_sessions_with_one_nickname() {
    let engine = setup_engine();

    let (sid1, _rx1) = engine
        .connect(
            Some("user-1".into()),
            "alice".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    let (sid2, _rx2) = engine
        .connect(Some("user-1".into()), "alice".into(), Protocol::Irc, None)
        .unwrap();

    assert!(engine.get_session(sid1).is_some());
    assert!(engine.get_session(sid2).is_some());
    assert_eq!(engine.user_connections.get("user-1").unwrap().len(), 2);
    engine.disconnect(sid2);
    assert_eq!(engine.get_session_id_by_nick("alice"), Some(sid1));
    assert_eq!(engine.user_connections.get("user-1").unwrap().len(), 1);
    engine.disconnect(sid1);
    assert!(engine.user_connections.get("user-1").is_none());
    assert!(engine.is_nick_available("alice"));
}

#[tokio::test]
async fn disconnecting_one_user_transport_does_not_emit_a_false_quit() {
    let engine = setup_engine();
    let (sid1, _rx1) = engine
        .connect(
            Some("user-1".into()),
            "alice".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    let (sid2, _rx2) = engine
        .connect(Some("user-1".into()), "alice".into(), Protocol::Irc, None)
        .unwrap();
    let (observer, mut observer_rx) = engine
        .connect(None, "observer".into(), Protocol::WebSocket, None)
        .unwrap();
    let mut channel = ChannelState::new(
        "channel".into(),
        DEFAULT_SERVER_ID.into(),
        "#general".into(),
    );
    channel.members.insert(sid1);
    channel.members.insert(sid2);
    channel.members.insert(observer);
    engine.channels.insert("channel".into(), channel);

    engine.disconnect(sid1);

    assert!(observer_rx.try_recv().is_err());
    assert_eq!(engine.user_connections.get("user-1").unwrap().len(), 1);
}

#[tokio::test]
async fn nickname_remains_exclusive_across_users() {
    let engine = setup_engine();
    engine
        .connect(
            Some("user-1".into()),
            "alice".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    assert!(
        engine
            .connect(Some("user-2".into()), "alice".into(), Protocol::Irc, None)
            .is_err()
    );
}

#[tokio::test]
async fn rfc1459_equivalent_nicknames_are_one_identity_slot() {
    let engine = setup_engine();
    engine
        .connect(Some("user-1".into()), "Nick[".into(), Protocol::Irc, None)
        .unwrap();
    assert!(
        engine
            .connect(Some("user-2".into()), "nICK{".into(), Protocol::Irc, None)
            .is_err()
    );
}

#[tokio::test]
async fn web_display_name_is_not_limited_by_irc_nickname_width() {
    let engine = setup_engine();
    let display_name = format!("{}-example.social", "long-handle".repeat(4));
    assert!(
        display_name.len() > crate::engine::validation::MAX_NICKNAME_LENGTH
            && engine
                .connect(
                    Some("user-1".into()),
                    display_name,
                    Protocol::WebSocket,
                    None,
                )
                .is_ok()
    );
}

#[tokio::test]
async fn dm_fans_out_to_every_active_recipient_connection() {
    let engine = setup_engine();
    let (sender, _sender_rx) = engine
        .connect(
            Some("alice-id".into()),
            "alice".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    let (_, mut web_rx) = engine
        .connect(
            Some("bob-id".into()),
            "bob".into(),
            Protocol::WebSocket,
            None,
        )
        .unwrap();
    let (_, mut irc_rx) = engine
        .connect(Some("bob-id".into()), "bob".into(), Protocol::Irc, None)
        .unwrap();

    engine
        .send_message(
            sender,
            DEFAULT_SERVER_ID,
            "bob",
            "both devices",
            None,
            None,
            None,
        )
        .unwrap();

    for receiver in [&mut web_rx, &mut irc_rx] {
        assert!(matches!(
            receiver.try_recv(),
            Ok(ChatEvent::Message { ref content, .. }) if content == "both devices"
        ));
    }
}

#[tokio::test]
async fn test_disconnect_nonexistent_session_is_noop() {
    let engine = setup_engine();
    let fake_id = ConnectionId::new();
    // Should not panic
    engine.disconnect(fake_id);
}

#[tokio::test]
async fn test_get_session_nonexistent() {
    let engine = setup_engine();
    assert!(engine.get_session(ConnectionId::new()).is_none());
}

#[tokio::test]
async fn test_is_nick_available() {
    let engine = setup_engine();
    assert!(engine.is_nick_available("alice"));
    let (_sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    assert!(!engine.is_nick_available("alice"));
    assert!(engine.is_nick_available("bob"));
}

#[tokio::test]
async fn test_connect_applies_protocol_specific_name_validation() {
    let engine = setup_engine();
    let result = engine.connect(None, "".into(), Protocol::WebSocket, None);
    assert!(result.is_err());

    let result = engine.connect(None, "has space!".into(), Protocol::WebSocket, None);
    assert!(result.is_ok());

    let result = engine.connect(None, "has space".into(), Protocol::Irc, None);
    assert!(result.is_err());

    let result = engine.connect(None, "1invalid".into(), Protocol::Irc, None);
    assert!(result.is_err());

    let result = engine.connect(None, "a".repeat(257), Protocol::WebSocket, None);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_connect_with_user_id_and_avatar() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(
            Some("user_123".into()),
            "alice".into(),
            Protocol::WebSocket,
            Some("https://example.com/avatar.png".into()),
        )
        .unwrap();
    let session = engine.get_session(sid).unwrap();
    assert_eq!(session.user_id.as_deref(), Some("user_123"));
    assert_eq!(
        session.avatar_url.as_deref(),
        Some("https://example.com/avatar.png")
    );
}

#[tokio::test]
async fn test_connect_irc_protocol() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "irc-user".into(), Protocol::Irc, None)
        .unwrap();
    let session = engine.get_session(sid).unwrap();
    assert_eq!(session.protocol, Protocol::Irc);
}

#[tokio::test]
async fn test_send_message_with_invalid_session() {
    let engine = setup_engine();
    let fake = ConnectionId::new();
    let result = engine.send_message(fake, DEFAULT_SERVER_ID, "#general", "hi", None, None, None);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_disconnect_broadcasts_quit() {
    let engine = setup_engine();
    let (sid_alice, _rx_alice) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let (sid_bob, mut rx_bob) = engine
        .connect(None, "bob".into(), Protocol::WebSocket, None)
        .unwrap();

    engine
        .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    engine
        .join_channel(sid_bob, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    while rx_bob.try_recv().is_ok() {}

    engine.disconnect(sid_alice);

    let event = rx_bob.try_recv().unwrap();
    match event {
        ChatEvent::Quit { nickname, .. } => assert_eq!(nickname, "alice"),
        _ => panic!("Expected Quit event, got {:?}", event),
    }
}

#[tokio::test]
async fn test_disconnect_removes_from_channel() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();

    engine.disconnect(sid);

    // Channel should have 0 members now
    let channels = engine.list_channels(DEFAULT_SERVER_ID);
    assert_eq!(channels[0].member_count, 0);
}
