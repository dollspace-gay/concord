use super::*;

#[test]
fn test_create_server() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_server",
        "name": "My Server",
        "icon_url": "https://example.com/icon.png"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateServer { name, icon_url } => {
            assert_eq!(name, "My Server");
            assert_eq!(icon_url, Some("https://example.com/icon.png".into()));
        }
        _ => panic!("Expected CreateServer"),
    }
}

#[test]
fn test_create_server_no_icon() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_server",
        "name": "My Server"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateServer { name, icon_url } => {
            assert_eq!(name, "My Server");
            assert!(icon_url.is_none());
        }
        _ => panic!("Expected CreateServer"),
    }
}

#[test]
fn test_delete_server() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "delete_server",
        "server_id": "srv-1"
    }"##,
    )
    .unwrap();
    assert!(matches!(msg, ClientMessage::DeleteServer { server_id } if server_id == "srv-1"));
}

#[test]
fn test_create_channel() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_channel",
        "server_id": "srv-1",
        "name": "new-channel"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateChannel {
            server_id,
            name,
            category_id,
            is_private,
            channel_type,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(name, "new-channel");
            assert!(category_id.is_none());
            assert!(is_private.is_none());
            assert!(channel_type.is_none());
        }
        _ => panic!("Expected CreateChannel"),
    }
}

#[test]
fn test_delete_channel() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "delete_channel",
        "server_id": "srv-1",
        "channel": "#old"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::DeleteChannel { server_id, channel } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(channel, "#old");
        }
        _ => panic!("Expected DeleteChannel"),
    }
}

#[test]
fn test_create_role() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_role",
        "server_id": "srv-1",
        "name": "Moderator",
        "color": "#ff0000",
        "permissions": 42
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateRole {
            server_id,
            name,
            color,
            permissions,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(name, "Moderator");
            assert_eq!(color, Some("#ff0000".into()));
            assert_eq!(permissions, Some(42));
        }
        _ => panic!("Expected CreateRole"),
    }
}

#[test]
fn test_create_category() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_category",
        "server_id": "srv-1",
        "name": "Text Channels"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateCategory { server_id, name } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(name, "Text Channels");
        }
        _ => panic!("Expected CreateCategory"),
    }
}

#[test]
fn test_create_thread() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_thread",
        "server_id": "srv-1",
        "parent_channel": "#general",
        "name": "Discussion",
        "message_id": "msg-1",
        "is_private": true
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateThread {
            server_id,
            parent_channel,
            name,
            message_id,
            is_private,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(parent_channel, "#general");
            assert_eq!(name, "Discussion");
            assert_eq!(message_id, "msg-1");
            assert!(is_private);
        }
        _ => panic!("Expected CreateThread"),
    }
}

#[test]
fn test_bulk_delete() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "bulk_delete_messages",
        "server_id": "srv-1",
        "channel": "#general",
        "message_ids": ["m1", "m2", "m3"]
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::BulkDeleteMessages { message_ids, .. } => {
            assert_eq!(message_ids.len(), 3);
        }
        _ => panic!("Expected BulkDeleteMessages"),
    }
}

#[test]
fn test_create_invite() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_invite",
        "server_id": "srv-1",
        "max_uses": 10,
        "expires_at": "2026-12-31T23:59:59Z"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateInvite {
            server_id,
            max_uses,
            expires_at,
            channel_id,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(max_uses, Some(10));
            assert_eq!(expires_at, Some("2026-12-31T23:59:59Z".into()));
            assert!(channel_id.is_none());
        }
        _ => panic!("Expected CreateInvite"),
    }
}

#[test]
fn test_create_event() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_event",
        "server_id": "srv-1",
        "name": "Game Night",
        "description": "Playing board games",
        "start_time": "2026-03-01T19:00:00Z"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateEvent {
            name,
            description,
            start_time,
            end_time,
            ..
        } => {
            assert_eq!(name, "Game Night");
            assert_eq!(description, Some("Playing board games".into()));
            assert_eq!(start_time, "2026-03-01T19:00:00Z");
            assert!(end_time.is_none());
        }
        _ => panic!("Expected CreateEvent"),
    }
}

#[test]
fn test_create_webhook() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_webhook",
        "server_id": "srv-1",
        "channel_id": "ch-1",
        "name": "GitHub Notifications",
        "webhook_type": "incoming",
        "url": "https://example.com/hook"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateWebhook {
            server_id,
            channel_id,
            name,
            webhook_type,
            url,
        } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(channel_id, "ch-1");
            assert_eq!(name, "GitHub Notifications");
            assert_eq!(webhook_type, "incoming");
            assert_eq!(url, Some("https://example.com/hook".into()));
        }
        _ => panic!("Expected CreateWebhook"),
    }
}

#[test]
fn test_create_bot() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_bot",
        "username": "mybot",
        "avatar_url": "https://example.com/bot.png"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateBot {
            username,
            avatar_url,
        } => {
            assert_eq!(username, "mybot");
            assert_eq!(avatar_url, Some("https://example.com/bot.png".into()));
        }
        _ => panic!("Expected CreateBot"),
    }
}

#[test]
fn test_create_bot_token() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_bot_token",
        "bot_user_id": "bot-1",
        "name": "production",
        "scopes": "read,write"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateBotToken {
            bot_user_id,
            name,
            scopes,
        } => {
            assert_eq!(bot_user_id, "bot-1");
            assert_eq!(name, "production");
            assert_eq!(scopes, Some("read,write".into()));
        }
        _ => panic!("Expected CreateBotToken"),
    }
}

#[test]
fn test_create_oauth2_app() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_o_auth2_app",
        "name": "My App",
        "description": "A cool app",
        "redirect_uris": ["https://example.com/callback"]
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateOAuth2App {
            name,
            description,
            redirect_uris,
            client_type,
        } => {
            assert_eq!(name, "My App");
            assert_eq!(description, Some("A cool app".into()));
            assert_eq!(redirect_uris, vec!["https://example.com/callback"]);
            assert_eq!(client_type, "confidential");
        }
        _ => panic!("Expected CreateOAuth2App"),
    }
}

#[test]
fn test_create_automod_rule() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_automod_rule",
        "server_id": "srv-1",
        "name": "No Spam",
        "rule_type": "keyword",
        "config": "{\"keywords\":[\"spam\"]}",
        "action_type": "delete"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateAutomodRule {
            name,
            rule_type,
            action_type,
            timeout_duration_seconds,
            ..
        } => {
            assert_eq!(name, "No Spam");
            assert_eq!(rule_type, "keyword");
            assert_eq!(action_type, "delete");
            assert!(timeout_duration_seconds.is_none());
        }
        _ => panic!("Expected CreateAutomodRule"),
    }
}

#[test]
fn test_update_community_settings() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "update_community_settings",
        "server_id": "srv-1",
        "description": "A cool server",
        "is_discoverable": true,
        "welcome_message": "Welcome!",
        "rules_text": "Be nice",
        "category": "gaming"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::UpdateCommunitySettings {
            is_discoverable,
            category,
            ..
        } => {
            assert!(is_discoverable);
            assert_eq!(category, Some("gaming".into()));
        }
        _ => panic!("Expected UpdateCommunitySettings"),
    }
}

#[test]
fn test_create_template() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "create_template",
        "server_id": "srv-1",
        "name": "Gaming Server",
        "description": "A template for gaming servers"
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::CreateTemplate {
            name, description, ..
        } => {
            assert_eq!(name, "Gaming Server");
            assert_eq!(description, Some("A template for gaming servers".into()));
        }
        _ => panic!("Expected CreateTemplate"),
    }
}

#[test]
fn test_update_notification_settings() {
    let msg: ClientMessage = parse_msg(
        r##"{
        "type": "update_notification_settings",
        "server_id": "srv-1",
        "level": "mentions_only",
        "suppress_everyone": true,
        "muted": false
    }"##,
    )
    .unwrap();
    match msg {
        ClientMessage::UpdateNotificationSettings {
            level,
            suppress_everyone,
            muted,
            ..
        } => {
            assert_eq!(level, "mentions_only");
            assert_eq!(suppress_everyone, Some(true));
            assert_eq!(muted, Some(false));
        }
        _ => panic!("Expected UpdateNotificationSettings"),
    }
}
