use super::*;

#[test]
fn test_normalize_channel_name() {
    assert_eq!(normalize_channel_name("#General"), "#general");
    assert_eq!(normalize_channel_name("general"), "#general");
    assert_eq!(normalize_channel_name("#rust"), "#rust");
}

#[tokio::test]
async fn test_part_channel() {
    let engine = setup_engine();

    let (sid1, mut rx1) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let (sid2, _rx2) = engine
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

    engine
        .part_channel(sid2, DEFAULT_SERVER_ID, "#general", None)
        .unwrap();

    let event = rx1.try_recv().unwrap();
    match event {
        ChatEvent::Part { nickname, .. } => assert_eq!(nickname, "bob"),
        _ => panic!("Expected Part event, got {:?}", event),
    }
}

#[tokio::test]
async fn test_set_topic() {
    let engine = setup_engine();

    let (sid, mut rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    while rx.try_recv().is_ok() {}

    engine
        .set_topic(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            "Welcome to Concord!".into(),
        )
        .await
        .unwrap();

    let event = rx.try_recv().unwrap();
    match event {
        ChatEvent::TopicChange { topic, .. } => {
            assert_eq!(topic, "Welcome to Concord!");
        }
        _ => panic!("Expected TopicChange event, got {:?}", event),
    }
}

#[tokio::test]
async fn test_join_channel_rejects_detached_creation() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let before = engine.list_channels(DEFAULT_SERVER_ID);
    assert!(
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#new-channel")
            .await
            .is_err()
    );
    assert_eq!(engine.list_channels(DEFAULT_SERVER_ID).len(), before.len());
}

#[tokio::test]
async fn test_join_channel_twice_is_ok() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    // Joining again should be a no-op, not an error
    let result = engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_part_channel_not_in() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    // Create the channel first by having someone join
    let (sid2, _rx2) = engine
        .connect(None, "bob".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    // alice never joined, so parting should fail
    let result = engine.part_channel(sid, DEFAULT_SERVER_ID, "#general", None);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_set_topic_too_long() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    let long_topic = "t".repeat(501);
    let result = engine
        .set_topic(sid, DEFAULT_SERVER_ID, "#general", long_topic)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_set_topic_empty_clears() {
    let engine = setup_engine();
    let (sid, mut rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    while rx.try_recv().is_ok() {}

    // Set a topic first
    engine
        .set_topic(sid, DEFAULT_SERVER_ID, "#general", "Hello".into())
        .await
        .unwrap();
    while rx.try_recv().is_ok() {}

    // Clear topic
    engine
        .set_topic(sid, DEFAULT_SERVER_ID, "#general", "".into())
        .await
        .unwrap();
    let event = rx.try_recv().unwrap();
    match event {
        ChatEvent::TopicChange { topic, .. } => {
            assert_eq!(topic, "");
        }
        _ => panic!("Expected TopicChange event"),
    }
}

#[test]
fn test_normalize_channel_name_already_lowercase() {
    assert_eq!(normalize_channel_name("#already-lower"), "#already-lower");
}

#[test]
fn test_normalize_channel_name_mixed_case() {
    assert_eq!(normalize_channel_name("MixedCase"), "#mixedcase");
}

#[test]
fn test_normalize_channel_name_uppercase_with_hash() {
    assert_eq!(normalize_channel_name("#UPPER"), "#upper");
}

#[test]
fn test_normalize_channel_name_with_numbers() {
    assert_eq!(normalize_channel_name("channel123"), "#channel123");
}

#[tokio::test]
async fn test_part_channel_with_reason() {
    let engine = setup_engine();
    let (sid_alice, mut rx_alice) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let (sid_bob, _rx_bob) = engine
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
    while rx_alice.try_recv().is_ok() {}

    engine
        .part_channel(
            sid_bob,
            DEFAULT_SERVER_ID,
            "#general",
            Some("bye!".to_string()),
        )
        .unwrap();

    let event = rx_alice.try_recv().unwrap();
    match event {
        ChatEvent::Part {
            nickname, reason, ..
        } => {
            assert_eq!(nickname, "bob");
            assert_eq!(reason, Some("bye!".to_string()));
        }
        _ => panic!("Expected Part event"),
    }
}

#[test]
fn test_resolve_channel_id_nonexistent() {
    let engine = setup_engine();
    let result = engine.resolve_channel_id(DEFAULT_SERVER_ID, "#nothing");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_resolve_channel_id_after_join() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid, DEFAULT_SERVER_ID, "#general")
        .await
        .unwrap();
    let result = engine.resolve_channel_id(DEFAULT_SERVER_ID, "#general");
    assert!(result.is_ok());
    assert!(!result.unwrap().is_empty());
}
