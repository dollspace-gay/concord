use super::*;

#[tokio::test]
async fn forum_commands_preserve_validation_denial_auth_and_dependency_fault_classes() {
    let (engine, pool, auth, credential_id, session_id, mut receiver) =
        forum_wire_fixture(true).await;
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"create_forum_tag","request_id":"invalid","server_id":"server","channel":"#forum","name":"","emoji":null,"moderated":false}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "INVALID_INPUT".into(),
            "forum tag name must contain 1 to 100 bytes".into(),
            false,
        )
    );

    sqlx::query(
        "CREATE TRIGGER reject_forum_tag BEFORE INSERT ON forum_tags \
         BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"create_forum_tag","request_id":"dependency","server_id":"server","channel":"#forum","name":"tag","emoji":null,"moderated":false}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "DEPENDENCY_UNAVAILABLE".into(),
            "dependency unavailable".into(),
            true,
        )
    );
    sqlx::query("DROP TRIGGER reject_forum_tag")
        .execute(&pool)
        .await
        .unwrap();
    auth.revoke_credential(&credential_id).await.unwrap();
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"create_forum_tag","request_id":"auth","server_id":"server","channel":"#forum","name":"tag","emoji":null,"moderated":false}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "UNAUTHENTICATED".into(),
            "authentication required".into(),
            false,
        )
    );

    let (engine, _, _, _, session_id, mut receiver) = forum_wire_fixture(false).await;
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"create_forum_tag","request_id":"denied","server_id":"server","channel":"#forum","name":"tag","emoji":null,"moderated":false}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "RESOURCE_UNAVAILABLE".into(),
            "resource unavailable".into(),
            false,
        )
    );
}

#[tokio::test]
async fn moderation_commands_preserve_validation_denial_auth_and_dependency_fault_classes() {
    let (engine, pool, auth, credential_id, session_id, mut receiver) =
        forum_wire_fixture(true).await;
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"ban_member","request_id":"invalid","server_id":"server","user_id":"member","delete_message_days":8}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "INVALID_INPUT".into(),
            "delete_message_days must be between 0 and 7".into(),
            false,
        )
    );

    sqlx::query(
        "CREATE TRIGGER reject_timeout_audit BEFORE INSERT ON audit_log \
         WHEN NEW.action_type='member_timeout' BEGIN SELECT RAISE(ABORT,'forced'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let timeout_until = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
    let dependency = serde_json::json!({
        "type": "timeout_member",
        "request_id": "dependency",
        "server_id": "server",
        "user_id": "member",
        "timeout_until": timeout_until,
    })
    .to_string();
    handle_client_message(&engine, session_id, &dependency).await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "DEPENDENCY_UNAVAILABLE".into(),
            "dependency unavailable".into(),
            true,
        )
    );
    sqlx::query("DROP TRIGGER reject_timeout_audit")
        .execute(&pool)
        .await
        .unwrap();
    auth.revoke_credential(&credential_id).await.unwrap();
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"kick_member","request_id":"auth","server_id":"server","user_id":"member"}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "UNAUTHENTICATED".into(),
            "authentication required".into(),
            false,
        )
    );

    let (engine, _, _, _, session_id, mut receiver) = forum_wire_fixture(false).await;
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"ban_member","request_id":"denied","server_id":"server","user_id":"owner","delete_message_days":0}"##,
    )
    .await;
    assert_eq!(
        receive_command_error(&mut receiver).await,
        (
            "RESOURCE_UNAVAILABLE".into(),
            "resource unavailable".into(),
            false,
        )
    );
}

#[test]
fn test_join_channel_default_server() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "join_channel",
        "channel": "#random"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::JoinChannel { server_id, .. } => {
            assert_eq!(server_id, DEFAULT_SERVER_ID);
        }
        _ => panic!("Expected JoinChannel"),
    }
}

#[test]
fn test_fetch_history_defaults() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "fetch_history",
        "channel": "#general"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::FetchHistory {
            server_id,
            before,
            limit,
            ..
        } => {
            assert_eq!(server_id, DEFAULT_SERVER_ID);
            assert!(before.is_none());
            assert!(limit.is_none());
        }
        _ => panic!("Expected FetchHistory"),
    }
}

#[test]
fn test_create_role_defaults() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_role",
        "server_id": "srv-1",
        "name": "Basic"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateRole {
            color, permissions, ..
        } => {
            assert!(color.is_none());
            assert!(permissions.is_none());
        }
        _ => panic!("Expected CreateRole"),
    }
}

#[test]
fn test_create_thread_defaults() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_thread",
        "server_id": "srv-1",
        "parent_channel": "#general",
        "name": "Public Thread",
        "message_id": "msg-2"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateThread { is_private, .. } => {
            assert!(!is_private);
        }
        _ => panic!("Expected CreateThread"),
    }
}

#[test]
fn test_ban_member_defaults() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "ban_member",
        "server_id": "srv-1",
        "user_id": "user-1"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::BanMember {
            delete_message_days,
            reason,
            ..
        } => {
            assert_eq!(delete_message_days, 0);
            assert!(reason.is_none());
        }
        _ => panic!("Expected BanMember"),
    }
}

#[test]
fn test_default_server_id() {
    assert_eq!(default_server_id(), DEFAULT_SERVER_ID);
}
