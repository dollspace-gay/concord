use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_accepts_exact_pong_then_disconnects_after_a_missed_probe() {
    let db = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&db).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('heartbeat-user','heartbeat')")
        .execute(&db)
        .await
        .unwrap();
    let auth = AuthService::new(db.clone(), "heartbeat-secret".into(), 1);
    let web_actor = auth.issue_web_session("heartbeat-user").await.unwrap().1;
    let token = auth
        .issue_irc_token(web_actor.user_id(), Some("heartbeat test"))
        .await
        .unwrap();
    let engine = Arc::new(ChatEngine::new(
        db.clone(),
        auth.clone(),
        "heartbeat-secret",
        4000,
        100,
    ));
    let (server, client) = tokio::io::duplex(16 * 1024);
    let cancel = CancellationToken::new();
    let task = tokio::spawn(handle_irc_connection_with_timing(
        server,
        "heartbeat-peer".into(),
        engine,
        db,
        auth,
        cancel,
        Duration::from_millis(100),
    ));
    let (reader, mut writer) = tokio::io::split(client);
    let mut reader = BufReader::new(reader);
    writer
        .write_all(
            format!(
                "PASS {}\r\nNICK heartbeat\r\nUSER heartbeat 0 * :Heartbeat\r\n",
                token.secret
            )
            .as_bytes(),
        )
        .await
        .unwrap();

    let first_nonce = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if let Some(nonce) = line.trim_end().strip_prefix("PING :") {
                break nonce.to_owned();
            }
        }
    })
    .await
    .expect("registered client did not receive heartbeat probe");
    writer
        .write_all(format!("PONG :{first_nonce}\r\n").as_bytes())
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if line.starts_with("PING :") {
                break;
            }
        }
    })
    .await
    .expect("exact PONG did not keep the connection alive for the next probe");

    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("client remained connected after missing the next heartbeat")
        .unwrap();
}

#[test]
fn test_part_event_with_reason() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Part {
            nickname: "bob".into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            reason: Some("goodbye".into()),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("PART #general"));
    assert!(lines[0].contains("goodbye"));
}

#[test]
fn test_part_event_no_reason() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Part {
            nickname: "bob".into(),
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            reason: None,
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("PART #general"));
}

#[test]
fn test_quit_event_with_reason() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Quit {
            nickname: "alice".into(),
            reason: Some("Leaving".into()),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("QUIT"));
    assert!(lines[0].contains("Leaving"));
}

#[test]
fn test_quit_event_no_reason() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Quit {
            nickname: "alice".into(),
            reason: None,
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("QUIT"));
}

#[test]
fn test_topic_change_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::TopicChange {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            set_by: "alice".into(),
            topic: "New topic".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("TOPIC #general"));
    assert!(lines[0].contains("New topic"));
}

#[test]
fn test_topic_event_with_content() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Topic {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#dev".into(),
            topic: "Development chat".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("#dev"));
    assert!(lines[0].contains("Development chat"));
}

#[test]
fn test_topic_event_empty() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Topic {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#dev".into(),
            topic: "".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    // Empty topic produces RPL_NOTOPIC
    assert!(lines[0].contains("331"));
}

#[test]
fn test_names_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Names {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            members: vec![
                MemberInfo {
                    nickname: "alice".into(),
                    avatar_url: None,
                    status: None,
                    custom_status: None,
                    status_emoji: None,
                    user_id: None,
                    server_avatar_url: None,
                    role_ids: Vec::new(),
                },
                MemberInfo {
                    nickname: "bob".into(),
                    avatar_url: None,
                    status: None,
                    custom_status: None,
                    status_emoji: None,
                    user_id: None,
                    server_avatar_url: None,
                    role_ids: Vec::new(),
                },
            ],
        },
    );
    // Names produces RPL_NAMREPLY + RPL_ENDOFNAMES
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("353"));
    assert!(lines[0].contains("alice"));
    assert!(lines[0].contains("bob"));
    assert!(lines[1].contains("366"));
}

#[test]
fn test_error_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::Error {
            code: "NOT_FOUND".into(),
            message: "Channel not found".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("NOTICE viewer"));
    assert!(lines[0].contains("[NOT_FOUND]"));
    assert!(lines[0].contains("Channel not found"));
}

#[test]
fn test_typing_start_is_silent() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::TypingStart {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            nickname: "alice".into(),
        },
    );
    assert!(lines.is_empty());
}

#[test]
fn test_ws_only_events_are_silent() {
    let engine = test_engine();

    let ws_events: Vec<ChatEvent> = vec![
        ChatEvent::History {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            messages: vec![],
            has_more: false,
        },
        ChatEvent::ServerList { servers: vec![] },
        ChatEvent::RoleList {
            server_id: DEFAULT_SERVER_ID.into(),
            version: 0,
            roles: vec![],
            member_roles: Some(vec![]),
        },
        ChatEvent::CategoryList {
            server_id: DEFAULT_SERVER_ID.into(),
            categories: vec![],
        },
        ChatEvent::PresenceList {
            server_id: DEFAULT_SERVER_ID.into(),
            presences: vec![],
        },
        ChatEvent::BookmarkList { bookmarks: vec![] },
        ChatEvent::InviteList {
            server_id: DEFAULT_SERVER_ID.into(),
            invites: vec![],
        },
        ChatEvent::TemplateList {
            server_id: DEFAULT_SERVER_ID.into(),
            templates: vec![],
        },
        ChatEvent::WebhookList {
            server_id: DEFAULT_SERVER_ID.into(),
            webhooks: vec![],
        },
    ];

    for event in &ws_events {
        let lines = event_to_irc_lines(&engine, "viewer", event);
        assert!(
            lines.is_empty(),
            "Expected no IRC output for {:?} but got {:?}",
            std::mem::discriminant(event),
            lines
        );
    }
}

#[test]
fn final_outbound_boundary_replaces_line_breaks_and_nul() {
    assert_eq!(
        sanitize_outbound_line(":alice PRIVMSG #general :one\r\nINJECT\0tail"),
        ":alice PRIVMSG #general :one  INJECT tail"
    );
}
