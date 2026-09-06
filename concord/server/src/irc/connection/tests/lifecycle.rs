use super::*;

#[test]
fn test_thread_create_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ThreadCreate {
            server_id: DEFAULT_SERVER_ID.into(),
            parent_channel: "#general".into(),
            thread: ThreadInfo {
                id: "thread-1".into(),
                name: "Discussion".into(),
                channel_type: "public_thread".into(),
                parent_message_id: None,
                creator_user_id: None,
                archived: false,
                state_version: 1,
                tags_version: 1,
                tag_ids: Vec::new(),
                auto_archive_minutes: 1440,
                message_count: 0,
                created_at: "2025-01-01".into(),
            },
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("Discussion"));
    assert!(lines[0].contains("thread"));
}

#[test]
fn test_thread_update_archived() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ThreadUpdate {
            server_id: DEFAULT_SERVER_ID.into(),
            thread: ThreadInfo {
                id: "thread-1".into(),
                name: "Old thread".into(),
                channel_type: "public_thread".into(),
                parent_message_id: None,
                creator_user_id: None,
                archived: true,
                state_version: 2,
                tags_version: 1,
                tag_ids: Vec::new(),
                auto_archive_minutes: 1440,
                message_count: 5,
                created_at: "2025-01-01".into(),
            },
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("archived"));
    assert!(lines[0].contains("Old thread"));
}

#[test]
fn test_thread_update_unarchived() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ThreadUpdate {
            server_id: DEFAULT_SERVER_ID.into(),
            thread: ThreadInfo {
                id: "thread-1".into(),
                name: "Revived thread".into(),
                channel_type: "public_thread".into(),
                parent_message_id: None,
                creator_user_id: None,
                archived: false,
                state_version: 3,
                tags_version: 1,
                tag_ids: Vec::new(),
                auto_archive_minutes: 1440,
                message_count: 10,
                created_at: "2025-01-01".into(),
            },
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("unarchived"));
}

#[test]
fn test_slow_mode_update_is_silent() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::SlowModeUpdate {
            server_id: DEFAULT_SERVER_ID.into(),
            channel: "#general".into(),
            seconds: 5,
        },
    );
    assert!(lines.is_empty());
}
