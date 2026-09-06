use super::*;

#[tokio::test]
async fn test_server_list_for_user() {
    let (engine, pool) = setup_engine().await;

    let user_id = create_test_user(&pool, "alice").await;

    // Create 3 servers
    let mut server_ids = Vec::new();
    for i in 0..3 {
        let sid = engine
            .create_server_for_actor(
                &actor_for(&pool, &user_id).await,
                format!("Server {i}"),
                None,
            )
            .await
            .unwrap();
        server_ids.push(sid);
    }

    // List servers for the user
    let servers = engine.list_servers_for_user(&user_id).await;
    assert_eq!(servers.len(), 3, "User should be a member of 3 servers");

    // Check each server has the correct role
    for server in &servers {
        assert_eq!(server.role, Some("owner".to_string()));
        assert_eq!(server.member_count, 1);
    }
}
