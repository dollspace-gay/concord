use super::*;

#[test]
fn test_event_json_has_type_tag() {
    let event = ChatEvent::Message {
        id: Uuid::new_v4().into(),
        server_id: None,
        conversation_id: None,
        from: "a".into(),
        target: "b".into(),
        content: "c".into(),
        timestamp: Utc::now(),
        avatar_url: None,
        reply_to: None,
        attachments: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"message""#));
}

#[test]
fn test_event_type_tags_are_snake_case() {
    let events: Vec<(ChatEvent, &str)> = vec![
        (
            ChatEvent::MessageEdit {
                id: Uuid::new_v4().into(),
                server_id: "s".into(),
                channel: "c".into(),
                content: "x".into(),
                edited_at: Utc::now(),
            },
            "message_edit",
        ),
        (
            ChatEvent::MessageDelete {
                id: Uuid::new_v4().into(),
                server_id: "s".into(),
                channel: "c".into(),
            },
            "message_delete",
        ),
        (
            ChatEvent::TopicChange {
                server_id: "s".into(),
                channel: "c".into(),
                set_by: "a".into(),
                topic: "t".into(),
            },
            "topic_change",
        ),
        (
            ChatEvent::NickChange {
                old_nick: "a".into(),
                new_nick: "b".into(),
            },
            "nick_change",
        ),
        (
            ChatEvent::ServerNotice {
                message: "hi".into(),
            },
            "server_notice",
        ),
        (
            ChatEvent::TypingStart {
                server_id: "s".into(),
                channel: "c".into(),
                nickname: "a".into(),
            },
            "typing_start",
        ),
        (
            ChatEvent::MemberKick {
                server_id: "s".into(),
                user_id: "u".into(),
                kicked_by: "a".into(),
                reason: None,
            },
            "member_kick",
        ),
        (
            ChatEvent::BulkMessageDelete {
                server_id: "s".into(),
                channel: "c".into(),
                message_ids: vec![],
            },
            "bulk_message_delete",
        ),
    ];

    for (event, expected_type) in events {
        let json = serde_json::to_string(&event).unwrap();
        let expected = format!(r#""type":"{}""#, expected_type);
        assert!(
            json.contains(&expected),
            "Event type tag should be '{}', got json: {}",
            expected_type,
            json
        );
    }
}

#[test]
fn test_all_info_structs_implement_debug() {
    // This test verifies Debug is implemented by using format!
    let _ = format!(
        "{:?}",
        ReplyInfo {
            id: "1".into(),
            from: "a".into(),
            content_preview: "b".into(),
        }
    );
    let _ = format!(
        "{:?}",
        ReactionGroup {
            emoji: "e".into(),
            count: 1,
            user_ids: vec![],
        }
    );
    let _ = format!(
        "{:?}",
        ServerInfo {
            id: "1".into(),
            name: "n".into(),
            icon_url: None,
            member_count: 0,
            role: None,
            my_permissions: 0,
        }
    );
    let _ = format!(
        "{:?}",
        ChannelInfo {
            id: "1".into(),
            conversation_id: "channel:31".into(),
            server_id: "s".into(),
            name: "n".into(),
            topic: "t".into(),
            member_count: 0,
            category_id: None,
            position: 0,
            is_private: false,
            channel_type: "text".into(),
            thread_parent_message_id: None,
            archived: false,
            slowmode_seconds: 0,
            is_nsfw: false,
        }
    );
    let _ = format!(
        "{:?}",
        MemberInfo {
            nickname: "n".into(),
            avatar_url: None,
            status: None,
            custom_status: None,
            status_emoji: None,
            user_id: None,
            server_avatar_url: None,
            role_ids: Vec::new(),
        }
    );
    let _ = format!(
        "{:?}",
        EmbedInfo {
            url: "u".into(),
            title: None,
            description: None,
            image_url: None,
            site_name: None,
        }
    );
    let _ = format!(
        "{:?}",
        WebhookInfo {
            id: "1".into(),
            server_id: "s".into(),
            channel_id: "c".into(),
            name: "n".into(),
            avatar_url: None,
            webhook_type: "incoming".into(),
            token: "t".into(),
            url: None,
            created_by: "u".into(),
            created_at: "d".into(),
        }
    );
    let _ = format!(
        "{:?}",
        BotTokenInfo {
            id: "1".into(),
            name: "n".into(),
            scopes: "bot".into(),
            created_at: "d".into(),
            last_used: None,
        }
    );
    let _ = format!(
        "{:?}",
        OAuth2AppInfo {
            id: "1".into(),
            name: "n".into(),
            description: "d".into(),
            icon_url: None,
            owner_id: "o".into(),
            redirect_uris: vec![],
            scopes: "identify".into(),
            is_public: false,
            created_at: "d".into(),
        }
    );
}

#[test]
fn test_all_info_structs_implement_clone() {
    let ri = ReplyInfo {
        id: "1".into(),
        from: "a".into(),
        content_preview: "b".into(),
    };
    let cloned = ri.clone();
    assert_eq!(cloned.id, "1");

    let wi = WebhookInfo {
        id: "1".into(),
        server_id: "s".into(),
        channel_id: "c".into(),
        name: "n".into(),
        avatar_url: None,
        webhook_type: "incoming".into(),
        token: "t".into(),
        url: None,
        created_by: "u".into(),
        created_at: "d".into(),
    };
    let cloned = wi.clone();
    assert_eq!(cloned.name, "n");
}
