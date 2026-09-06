use super::*;

#[tokio::test]
async fn test_create_server() {
    let engine = setup_engine();

    let server_id = engine
        .create_server("Test Server".into(), "user1".into(), None)
        .await
        .unwrap();

    assert!(engine.servers.contains_key(&server_id));
    let channels = engine.list_channels(&server_id);
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].name, "#general");
}

#[tokio::test]
async fn test_server_isolation() {
    let engine = setup_engine();

    let server_a = engine
        .create_server("Server A".into(), "user1".into(), None)
        .await
        .unwrap();
    let server_b = engine
        .create_server("Server B".into(), "user1".into(), None)
        .await
        .unwrap();

    let (sid, mut rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();

    engine
        .join_channel(sid, &server_a, "#general")
        .await
        .unwrap();
    while rx.try_recv().is_ok() {}

    let (sid2, _rx2) = engine
        .connect(None, "bob".into(), Protocol::WebSocket, None)
        .unwrap();
    engine
        .join_channel(sid2, &server_b, "#general")
        .await
        .unwrap();

    // Alice is not in server_b's #general — should fail
    let result = engine.send_message(sid, &server_b, "#general", "Hello", None, None, None);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_join_channel_nonexistent_server() {
    let engine = setup_engine();
    let (sid, _rx) = engine
        .connect(None, "alice".into(), Protocol::WebSocket, None)
        .unwrap();
    let result = engine.join_channel(sid, "no-such-server", "#general").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_create_server_invalid_name() {
    let engine = setup_engine();
    // Empty name
    let result = engine.create_server("".into(), "user1".into(), None).await;
    assert!(result.is_err());

    // Whitespace-only name
    let result = engine
        .create_server("   ".into(), "user1".into(), None)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_create_server_too_long_name() {
    let engine = setup_engine();
    let long_name = "a".repeat(101);
    let result = engine.create_server(long_name, "user1".into(), None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_create_server_max_name_length() {
    let engine = setup_engine();
    let max_name = "a".repeat(100);
    let result = engine.create_server(max_name, "user1".into(), None).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_create_server_with_icon() {
    let engine = setup_engine();
    let server_id = engine
        .create_server(
            "My Server".into(),
            "user1".into(),
            Some("https://example.com/icon.png".into()),
        )
        .await
        .unwrap();
    let server = engine.servers.get(&server_id).unwrap();
    assert_eq!(
        server.icon_url.as_deref(),
        Some("https://example.com/icon.png")
    );
}

#[tokio::test]
async fn test_find_server_by_name() {
    let engine = setup_engine();
    let server_id = engine
        .create_server("Test Server".into(), "user1".into(), None)
        .await
        .unwrap();
    // Case insensitive lookup
    assert_eq!(
        engine.find_server_by_name("test server"),
        Some(server_id.clone())
    );
    assert_eq!(engine.find_server_by_name("TEST SERVER"), Some(server_id));
    assert!(engine.find_server_by_name("nonexistent").is_none());
}

#[tokio::test]
async fn test_get_server_name() {
    let engine = setup_engine();
    let server_id = engine
        .create_server("My Server".into(), "user1".into(), None)
        .await
        .unwrap();
    assert_eq!(
        engine.get_server_name(&server_id),
        Some("My Server".to_string())
    );
    assert!(engine.get_server_name("nonexistent").is_none());
}

#[tokio::test]
async fn test_create_server_sets_owner() {
    let engine = setup_engine();
    let server_id = engine
        .create_server("Test".into(), "user1".into(), None)
        .await
        .unwrap();
    let server = engine.servers.get(&server_id).unwrap();
    assert_eq!(server.owner_id, "user1");
}

#[tokio::test]
async fn test_join_server_nonexistent() {
    let engine = setup_engine();
    let result = engine.join_server("user1", "nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_join_server_adds_member() {
    let engine = setup_engine();
    let server_id = engine
        .create_server("Test".into(), "owner".into(), None)
        .await
        .unwrap();
    engine.join_server("user1", &server_id).await.unwrap();
    let server = engine.servers.get(&server_id).unwrap();
    assert!(server.member_user_ids.contains("user1"));
}

#[tokio::test]
async fn test_leave_server_removes_member() {
    let engine = setup_engine();
    let server_id = engine
        .create_server("Test".into(), "owner".into(), None)
        .await
        .unwrap();
    engine.join_server("user1", &server_id).await.unwrap();
    engine.leave_server("user1", &server_id).await.unwrap();
    let server = engine.servers.get(&server_id).unwrap();
    assert!(!server.member_user_ids.contains("user1"));
}

#[tokio::test]
async fn test_list_servers_for_user() {
    let engine = setup_engine();
    let sid1 = engine
        .create_server("Server A".into(), "user1".into(), None)
        .await
        .unwrap();
    let sid2 = engine
        .create_server("Server B".into(), "user1".into(), None)
        .await
        .unwrap();
    let _ = engine
        .create_server("Server C".into(), "user2".into(), None)
        .await
        .unwrap();

    // user1 should see Server A and Server B (they're the owner)
    engine.join_server("user1", &sid1).await.unwrap();
    engine.join_server("user1", &sid2).await.unwrap();
    let servers = engine.list_servers_for_user("user1").await;
    assert_eq!(servers.len(), 2);
    let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Server A"));
    assert!(names.contains(&"Server B"));
}

#[tokio::test]
async fn test_server_ids_are_unique() {
    let engine = setup_engine();
    let id1 = engine
        .create_server("S1".into(), "user1".into(), None)
        .await
        .unwrap();
    let id2 = engine
        .create_server("S2".into(), "user1".into(), None)
        .await
        .unwrap();
    assert_ne!(id1, id2);
}
