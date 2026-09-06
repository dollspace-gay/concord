use super::*;

#[test]
fn test_join_event_roundtrip() {
    let event = ChatEvent::Join {
        nickname: "alice".into(),
        server_id: "srv1".into(),
        channel: "#general".into(),
        avatar_url: Some("https://example.com/avatar.png".into()),
        user_id: Some("user-alice".into()),
        server_avatar_url: None,
        role_ids: vec!["role-member".into()],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::Join {
            nickname,
            avatar_url,
            ..
        } => {
            assert_eq!(nickname, "alice");
            assert!(avatar_url.is_some());
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_part_event_roundtrip() {
    let event = ChatEvent::Part {
        nickname: "alice".into(),
        server_id: "srv1".into(),
        channel: "#general".into(),
        reason: Some("goodbye".into()),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::Part {
            nickname, reason, ..
        } => {
            assert_eq!(nickname, "alice");
            assert_eq!(reason, Some("goodbye".into()));
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_quit_event_roundtrip() {
    let event = ChatEvent::Quit {
        nickname: "alice".into(),
        reason: None,
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::Quit { nickname, reason } => {
            assert_eq!(nickname, "alice");
            assert!(reason.is_none());
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_topic_change_event_roundtrip() {
    let event = ChatEvent::TopicChange {
        server_id: "srv1".into(),
        channel: "#general".into(),
        set_by: "alice".into(),
        topic: "New topic".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::TopicChange { topic, set_by, .. } => {
            assert_eq!(topic, "New topic");
            assert_eq!(set_by, "alice");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_server_notice_event_roundtrip() {
    let event = ChatEvent::ServerNotice {
        message: "Welcome!".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::ServerNotice { message } => {
            assert_eq!(message, "Welcome!");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_error_event_roundtrip() {
    let event = ChatEvent::Error {
        code: "FORBIDDEN".into(),
        message: "No permission".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::Error { code, message } => {
            assert_eq!(code, "FORBIDDEN");
            assert_eq!(message, "No permission");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_typing_start_event_roundtrip() {
    let event = ChatEvent::TypingStart {
        server_id: "srv1".into(),
        channel: "#general".into(),
        nickname: "alice".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::TypingStart { nickname, .. } => {
            assert_eq!(nickname, "alice");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_member_kick_event_roundtrip() {
    let event = ChatEvent::MemberKick {
        server_id: "srv1".into(),
        user_id: "user1".into(),
        kicked_by: "admin1".into(),
        reason: Some("violated rules".into()),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::MemberKick {
            user_id,
            kicked_by,
            reason,
            ..
        } => {
            assert_eq!(user_id, "user1");
            assert_eq!(kicked_by, "admin1");
            assert_eq!(reason, Some("violated rules".into()));
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_member_ban_event_roundtrip() {
    let event = ChatEvent::MemberBan {
        server_id: "srv1".into(),
        user_id: "user1".into(),
        banned_by: "admin1".into(),
        reason: None,
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::MemberBan {
            user_id, reason, ..
        } => {
            assert_eq!(user_id, "user1");
            assert!(reason.is_none());
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_member_timeout_event_roundtrip() {
    let event = ChatEvent::MemberTimeout {
        server_id: "srv1".into(),
        user_id: "user1".into(),
        timeout_until: Some("2026-03-01T00:00:00Z".into()),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::MemberTimeout {
            user_id,
            timeout_until,
            ..
        } => {
            assert_eq!(user_id, "user1");
            assert_eq!(timeout_until, Some("2026-03-01T00:00:00Z".into()));
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_discover_servers_event_roundtrip() {
    let event = ChatEvent::DiscoverServers {
        servers: vec![ServerCommunityInfo {
            server_id: "srv1".into(),
            description: Some("A fun server".into()),
            is_discoverable: true,
            welcome_message: Some("Welcome!".into()),
            rules_text: None,
            category: Some("gaming".into()),
            rules_accepted: None,
        }],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::DiscoverServers { servers } => {
            assert_eq!(servers.len(), 1);
            assert!(servers[0].is_discoverable);
            assert_eq!(servers[0].category, Some("gaming".into()));
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_button_component_roundtrip() {
    let component = MessageComponent::Button {
        custom_id: "btn1".into(),
        label: "Click me".into(),
        style: "primary".into(),
        emoji: Some("\u{1F44D}".into()),
        disabled: false,
    };
    let json = serde_json::to_string(&component).unwrap();
    let restored: MessageComponent = serde_json::from_str(&json).unwrap();
    match restored {
        MessageComponent::Button {
            custom_id, label, ..
        } => {
            assert_eq!(custom_id, "btn1");
            assert_eq!(label, "Click me");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_select_menu_component_roundtrip() {
    let component = MessageComponent::SelectMenu {
        custom_id: "select1".into(),
        placeholder: Some("Choose...".into()),
        options: vec![SelectOption {
            label: "Option A".into(),
            value: "a".into(),
            description: Some("First option".into()),
            emoji: None,
            default: true,
        }],
        min_values: 1,
        max_values: 3,
    };
    let json = serde_json::to_string(&component).unwrap();
    let restored: MessageComponent = serde_json::from_str(&json).unwrap();
    match restored {
        MessageComponent::SelectMenu {
            options,
            max_values,
            ..
        } => {
            assert_eq!(options.len(), 1);
            assert!(options[0].default);
            assert_eq!(max_values, 3);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_action_row_component_roundtrip() {
    let component = MessageComponent::ActionRow {
        components: vec![
            MessageComponent::Button {
                custom_id: "btn1".into(),
                label: "Yes".into(),
                style: "primary".into(),
                emoji: None,
                disabled: false,
            },
            MessageComponent::Button {
                custom_id: "btn2".into(),
                label: "No".into(),
                style: "danger".into(),
                emoji: None,
                disabled: false,
            },
        ],
    };
    let json = serde_json::to_string(&component).unwrap();
    let restored: MessageComponent = serde_json::from_str(&json).unwrap();
    match restored {
        MessageComponent::ActionRow { components } => {
            assert_eq!(components.len(), 2);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_interaction_response_data_roundtrip() {
    let resp = InteractionResponseData {
        content: Some("Hello!".into()),
        embeds: Some(vec![RichEmbedInfo {
            title: Some("Title".into()),
            description: Some("Desc".into()),
            url: None,
            color: Some("#FF0000".into()),
            fields: Some(vec![EmbedField {
                name: "Field 1".into(),
                value: "Value 1".into(),
                inline: true,
            }]),
            footer: Some(EmbedFooter {
                text: "Footer text".into(),
                icon_url: None,
            }),
            image_url: None,
            thumbnail_url: None,
            author: Some(EmbedAuthor {
                name: "Author".into(),
                url: None,
                icon_url: None,
            }),
            timestamp: None,
        }]),
        components: None,
        ephemeral: true,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let restored: InteractionResponseData = serde_json::from_str(&json).unwrap();
    assert!(restored.ephemeral);
    assert_eq!(restored.content, Some("Hello!".into()));
    let embeds = restored.embeds.unwrap();
    assert_eq!(embeds.len(), 1);
    assert_eq!(embeds[0].title, Some("Title".into()));
    let fields = embeds[0].fields.as_ref().unwrap();
    assert_eq!(fields.len(), 1);
    assert!(fields[0].inline);
}

#[test]
fn test_bluesky_share_result_roundtrip() {
    let event = ChatEvent::BlueskyShareResult {
        message_id: "msg1".into(),
        success: true,
        post_uri: Some("at://did:plc:abc/app.bsky.feed.post/xyz".into()),
        error: None,
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::BlueskyShareResult {
            message_id,
            success,
            post_uri,
            error,
        } => {
            assert_eq!(message_id, "msg1");
            assert!(success);
            assert!(post_uri.is_some());
            assert!(error.is_none());
        }
        _ => panic!("Wrong variant"),
    }
}
