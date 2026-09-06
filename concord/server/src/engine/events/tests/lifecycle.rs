use super::*;

#[test]
fn test_slow_mode_update_event_roundtrip() {
    let event = ChatEvent::SlowModeUpdate {
        server_id: "srv1".into(),
        channel: "#general".into(),
        seconds: 10,
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::SlowModeUpdate { seconds, .. } => {
            assert_eq!(seconds, 10);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_interaction_create_event_roundtrip() {
    let event = ChatEvent::InteractionCreate {
        interaction: InteractionInfo {
            id: "int1".into(),
            interaction_type: "slash_command".into(),
            command_name: Some("ping".into()),
            user_id: "user1".into(),
            server_id: "srv1".into(),
            channel_id: "ch1".into(),
            data: serde_json::json!({"target": "user2"}),
        },
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::InteractionCreate { interaction } => {
            assert_eq!(interaction.id, "int1");
            assert_eq!(interaction.command_name, Some("ping".into()));
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_invite_create_event_roundtrip() {
    let event = ChatEvent::InviteCreate {
        server_id: "srv1".into(),
        invite: InviteInfo {
            id: "inv1".into(),
            code: "abc12345".into(),
            server_id: "srv1".into(),
            created_by: "user1".into(),
            max_uses: Some(10),
            use_count: 0,
            expires_at: Some("2026-12-31T23:59:59Z".into()),
            channel_id: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        },
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::InviteCreate { invite, .. } => {
            assert_eq!(invite.code, "abc12345");
            assert_eq!(invite.max_uses, Some(10));
            assert_eq!(invite.use_count, 0);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_event_update_event_roundtrip() {
    let event = ChatEvent::EventUpdate {
        server_id: "srv1".into(),
        event: EventInfo {
            id: "ev1".into(),
            server_id: "srv1".into(),
            name: "Game Night".into(),
            description: Some("Weekly game night".into()),
            channel_id: None,
            start_time: "2026-03-01T20:00:00Z".into(),
            end_time: Some("2026-03-01T23:00:00Z".into()),
            image_url: None,
            created_by: "user1".into(),
            status: "scheduled".into(),
            interested_count: 5,
            created_at: "2026-01-01T00:00:00Z".into(),
        },
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::EventUpdate { event: ei, .. } => {
            assert_eq!(ei.name, "Game Night");
            assert_eq!(ei.status, "scheduled");
            assert_eq!(ei.interested_count, 5);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_thread_create_event_roundtrip() {
    let event = ChatEvent::ThreadCreate {
        server_id: "srv1".into(),
        parent_channel: "#general".into(),
        thread: ThreadInfo {
            id: "thread1".into(),
            name: "#my-thread".into(),
            channel_type: "public_thread".into(),
            parent_message_id: Some("msg1".into()),
            creator_user_id: Some("user1".into()),
            archived: false,
            state_version: 1,
            tags_version: 1,
            tag_ids: Vec::new(),
            auto_archive_minutes: 1440,
            message_count: 0,
            created_at: "2026-01-01T00:00:00Z".into(),
        },
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::ThreadCreate { thread, .. } => {
            assert_eq!(thread.name, "#my-thread");
            assert_eq!(thread.channel_type, "public_thread");
            assert!(!thread.archived);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_presence_update_event_roundtrip() {
    let event = ChatEvent::PresenceUpdate {
        server_id: "srv1".into(),
        presence: PresenceInfo {
            user_id: "user1".into(),
            nickname: "alice".into(),
            avatar_url: None,
            status: "online".into(),
            custom_status: Some("Coding!".into()),
            status_emoji: Some("\u{1F4BB}".into()),
        },
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::PresenceUpdate { presence, .. } => {
            assert_eq!(presence.status, "online");
            assert_eq!(presence.custom_status, Some("Coding!".into()));
        }
        _ => panic!("Wrong variant"),
    }
}
