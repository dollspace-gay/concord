use super::*;

#[tokio::test]
async fn test_join_and_message() {
    let engine = setup_engine();

    let (sid1, mut rx1) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let (sid2, mut rx2) = engine
        .connect(None, "bob".into(), Protocol::WebSocket, None)
        .unwrap();

    engine
        .join_channel(sid1, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    engine
        .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();

    while rx1.try_recv().is_ok() {}
    while rx2.try_recv().is_ok() {}

    engine
        .send_message(
            sid1,
            DEFAULT_SERVER_ID,
            "#general",
            "Hello from Alice!",
            None,
            None,
            None,
        )
        .unwrap();

    let event = rx2.try_recv().unwrap();
    match event {
        ChatEvent::Message { from, content, .. } => {
            assert_eq!(from, "alice");
            assert_eq!(content, "Hello from Alice!");
        }
        _ => panic!("Expected Message event, got {:?}", event),
    }

    // Sender receives a MessageAck (not the Message itself)
    let ack = rx1.try_recv().unwrap();
    assert!(matches!(ack, ChatEvent::MessageAck { .. }));
    assert!(rx1.try_recv().is_err());
}

#[tokio::test]
async fn test_dm() {
    let engine = setup_engine();

    let (sid1, _rx1) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let (_sid2, mut rx2) = engine
        .connect(None, "bob".into(), Protocol::WebSocket, None)
        .unwrap();

    engine
        .send_message(sid1, DEFAULT_SERVER_ID, "bob", "Hey Bob!", None, None, None)
        .unwrap();

    let event = rx2.try_recv().unwrap();
    match event {
        ChatEvent::Message {
            from,
            target,
            content,
            ..
        } => {
            assert_eq!(from, "alice");
            assert_eq!(target, "bob");
            assert_eq!(content, "Hey Bob!");
        }
        _ => panic!("Expected Message event, got {:?}", event),
    }
}

#[tokio::test]
async fn test_list_channels() {
    let engine = setup_engine();

    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#rust")
        .await
        .unwrap();

    let channels = engine.list_channels(DEFAULT_SERVER_ID);
    assert_eq!(channels.len(), 2);

    let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"#general"));
    assert!(names.contains(&"#rust"));
}

#[tokio::test]
async fn test_send_message_to_nonexistent_channel() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let result = engine.send_message(
        sid,
        DEFAULT_SERVER_ID,
        "#nonexistent",
        "hello",
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[tokio::test]
async fn test_channel_name_normalization_on_join() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    // Joining with "General" should normalize to "#general"
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "General")
        .await
        .unwrap();
    let channels = engine.list_channels(DEFAULT_SERVER_ID);
    assert!(channels.iter().any(|channel| channel.name == "#general"));
}

#[tokio::test]
async fn test_message_not_echoed_to_sender() {
    let engine = setup_engine();
    let (sid, mut rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    // Drain join events
    while rx.try_recv().is_ok() {}

    engine
        .send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            "hello",
            None,
            None,
            None,
        )
        .unwrap();
    // Message should NOT be echoed back to the sender (only a MessageAck)
    let event = rx.try_recv().unwrap();
    assert!(matches!(event, ChatEvent::MessageAck { .. }));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_empty_message_rejected() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    let result = engine.send_message(sid, DEFAULT_SERVER_ID, "#general", "", None, None, None);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_whitespace_only_message_rejected() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    let result = engine.send_message(sid, DEFAULT_SERVER_ID, "#general", "   ", None, None, None);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_oversized_message_rejected() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    let long_msg = "x".repeat(4001);
    let result = engine.send_message(
        sid,
        DEFAULT_SERVER_ID,
        "#general",
        &long_msg,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[tokio::test]
async fn test_message_at_max_length_accepted() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    let max_msg = "x".repeat(4000);
    let result = engine.send_message(
        sid,
        DEFAULT_SERVER_ID,
        "#general",
        &max_msg,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_multiple_channels_message_isolation() {
    let engine = setup_engine();
    let (sid_alice, mut rx_alice) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let (sid_bob, mut rx_bob) = engine
        .connect(None, "bob".into(), Protocol::WebSocket, None)
        .unwrap();

    // Alice joins #general, Bob joins #rust
    engine
        .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    engine
        .join_channel(sid_bob, DEFAULT_SERVER_ID, "#rust")
        .await
        .unwrap();
    while rx_alice.try_recv().is_ok() {}
    while rx_bob.try_recv().is_ok() {}

    // Alice sends to #general — Bob should NOT receive it (different channel)
    engine
        .send_message(
            sid_alice,
            DEFAULT_SERVER_ID,
            "#general",
            "hello general",
            None,
            None,
            None,
        )
        .unwrap();
    assert!(rx_bob.try_recv().is_err());
}

#[tokio::test]
async fn test_dm_to_nonexistent_user() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let result = engine.send_message(sid, DEFAULT_SERVER_ID, "nobody", "hello", None, None, None);
    // DMs to non-existent users fail because there's no channel and no user session
    assert!(result.is_err());
}

#[tokio::test]
async fn test_message_rate_limiting() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();

    // The default rate limiter allows burst of 10.
    // Send 10 messages — all should succeed.
    for i in 0..10 {
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            &format!("msg {i}"),
            None,
            None,
            None,
        );
        assert!(result.is_ok(), "Message {i} should succeed");
    }

    // 11th should be rate-limited
    let result = engine.send_message(
        sid,
        DEFAULT_SERVER_ID,
        "#general",
        "msg 10",
        None,
        None,
        None,
    );
    assert!(result.is_err());
}
