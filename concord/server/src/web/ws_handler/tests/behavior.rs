use super::*;

#[test]
fn bootstrap_reads_do_not_consume_mutation_admission() {
    let before = crate::runtime_metrics::snapshot();
    let admission_index = crate::runtime_metrics::Operation::CommandAdmission as usize;
    let mut read_count = 0;
    let mut read_window = Instant::now();
    let mut mutation_count = 0;
    let mut mutation_window = Instant::now();

    for _ in 0..120 {
        assert!(fixed_window_admit(&mut read_count, &mut read_window, 120));
    }
    assert!(!fixed_window_admit(&mut read_count, &mut read_window, 120));
    assert!(fixed_window_admit(
        &mut mutation_count,
        &mut mutation_window,
        30
    ));
    let after = crate::runtime_metrics::snapshot();
    assert!(after.succeeded[admission_index] >= before.succeeded[admission_index] + 121);
    assert!(after.failed[admission_index] > before.failed[admission_index]);
}

#[test]
fn websocket_admission_classifies_reads_and_preserves_correlation() {
    assert!(websocket_command_is_read(
        r#"{"type":"list_channels","server_id":"server"}"#
    ));
    assert!(websocket_command_is_read(
        r#"{"type":"sync","request_id":"sync-1","protocol_version":2,"subscriptions":[]}"#
    ));
    assert!(websocket_command_is_read(r#"{"type":"list_owned_bots"}"#));
    assert!(!websocket_command_is_read(
        r#"{"type":"set_presence","status":"idle"}"#
    ));
    assert!(!websocket_command_is_read("not json"));
    assert_eq!(
        websocket_command_correlation(
            r#"{"type":"sync","request_id":"sync-1","protocol_version":2,"subscriptions":[]}"#
        )
        .as_deref(),
        Some("sync-1")
    );
}

#[tokio::test]
async fn lifecycle_mutation_reports_success_only_after_command_acceptance() {
    let (engine, pool, _, _, session_id, mut receiver) = forum_wire_fixture(true).await;
    handle_client_message(
        &engine,
        session_id,
        r##"{"type":"lifecycle_command","request_id":"create-tag","command":{"type":"create_forum_tag","server_id":"server","channel":"#forum","name":"accepted","emoji":null,"moderated":false}}"##,
    )
    .await;

    assert_eq!(receive_lifecycle_success(&mut receiver).await, "create-tag");
    let persisted: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM forum_tags WHERE name='accepted'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(persisted, 1);
}

#[test]
fn protocol_v2_mutation_requires_operation_generation() {
    assert!(parse_msg(
        r##"{"type":"send_message","request_id":"request-1","channel":"#general","content":"hello"}"##
    )
    .is_err());
}

#[test]
fn test_set_topic() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "set_topic",
        "server_id": "srv-1",
        "channel": "#general",
        "topic": "New topic"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SetTopic { topic, .. } => {
            assert_eq!(topic, "New topic");
        }
        _ => panic!("Expected SetTopic"),
    }
}

#[test]
fn test_typing() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "typing",
        "channel": "#general"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::Typing { server_id, channel } => {
            assert_eq!(server_id, DEFAULT_SERVER_ID);
            assert_eq!(channel, "#general");
        }
        _ => panic!("Expected Typing"),
    }
}

#[test]
fn test_mark_read() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "mark_read",
        "operation_generation": "generation-0001",
        "server_id": "srv-1",
        "channel": "#general",
        "message_id": "msg-42"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::MarkRead {
            server_id,
            channel,
            message_id,
            ..
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(channel, "#general");
            assert_eq!(message_id, "msg-42");
        }
        _ => panic!("Expected MarkRead"),
    }
}

#[test]
fn test_set_presence() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "set_presence",
        "status": "dnd",
        "custom_status": "In a meeting",
        "status_emoji": "\ud83d\udcbc"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SetPresence {
            status,
            custom_status,
            status_emoji,
        } => {
            assert_eq!(status, "dnd");
            assert_eq!(custom_status, Some("In a meeting".into()));
            assert_eq!(status_emoji, Some("\u{1f4bc}".into()));
        }
        _ => panic!("Expected SetPresence"),
    }
}

#[test]
fn test_add_bookmark() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "add_bookmark",
        "message_id": "msg-1",
        "note": "Important info"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::AddBookmark { message_id, note } => {
            assert_eq!(message_id, "msg-1");
            assert_eq!(note, Some("Important info".into()));
        }
        _ => panic!("Expected AddBookmark"),
    }
}

#[test]
fn test_set_slow_mode() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "set_slow_mode",
        "server_id": "srv-1",
        "channel": "#general",
        "seconds": 10
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SetSlowMode { seconds, .. } => {
            assert_eq!(seconds, 10);
        }
        _ => panic!("Expected SetSlowMode"),
    }
}

#[test]
fn test_use_invite() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "use_invite",
        "code": "abc123"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::UseInvite { code } if code == "abc123"));
}

#[test]
fn test_register_slash_command() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "register_slash_command",
        "server_id": "srv-1",
        "name": "ping",
        "description": "Check if bot is alive",
        "options_json": "[{\"name\":\"target\",\"type\":\"string\"}]"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::RegisterSlashCommand {
            server_id,
            name,
            description,
            options_json,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(name, "ping");
            assert_eq!(description, "Check if bot is alive");
            assert!(options_json.is_some());
        }
        _ => panic!("Expected RegisterSlashCommand"),
    }
}

#[test]
fn test_invoke_slash_command() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "invoke_slash_command",
        "request_id": "request-1",
        "server_id": "srv-1",
        "channel": "#general",
        "command_name": "ping"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::InvokeSlashCommand {
            command_name,
            args_json,
            ..
        } => {
            assert_eq!(command_name, "ping");
            assert!(args_json.is_none());
        }
        _ => panic!("Expected InvokeSlashCommand"),
    }
}

#[test]
fn test_respond_to_interaction() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "respond_to_interaction",
        "interaction_id": "int-1",
        "content": "Pong!",
        "ephemeral": true
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::RespondToInteraction {
            interaction_id,
            content,
            ephemeral,
            ..
        } => {
            assert_eq!(interaction_id, "int-1");
            assert_eq!(content, Some("Pong!".into()));
            assert_eq!(ephemeral, Some(true));
        }
        _ => panic!("Expected RespondToInteraction"),
    }
}

#[test]
fn test_malformed_json_missing_type() {
    assert!(parse_msg(r##"{"channel": "#general"}"##).is_err());
}

#[test]
fn test_malformed_json_unknown_type() {
    assert!(parse_msg(r##"{"type": "unknown_command"}"##).is_err());
}

#[test]
fn test_malformed_json_missing_required_field() {
    // SendMessage requires channel and content
    assert!(parse_msg(r##"{"type": "send_message"}"##).is_err());
}

#[test]
fn test_malformed_json_wrong_field_type() {
    // limit should be a number, not a string
    assert!(
        parse_msg(
            r##"{
        "type": "fetch_history",
        "channel": "#general",
        "limit": "not a number"
    }"##
        )
        .is_err()
    );
}

#[test]
fn test_extra_fields_ignored() {
    // Extra fields should be silently ignored by serde
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "list_servers",
        "unknown_field": "should be ignored",
        "another_extra": 42
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::ListServers));
}

#[test]
fn test_empty_json_object() {
    assert!(parse_msg("{}").is_err());
}

#[test]
fn test_null_type() {
    assert!(parse_msg(r##"{"type": null}"##).is_err());
}

#[test]
fn test_set_nsfw() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "set_nsfw",
        "server_id": "srv-1",
        "channel": "#mature",
        "is_nsfw": true
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::SetNsfw { is_nsfw, .. } => {
            assert!(is_nsfw);
        }
        _ => panic!("Expected SetNsfw"),
    }
}
