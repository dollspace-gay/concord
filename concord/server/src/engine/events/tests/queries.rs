use super::*;

#[test]
fn test_webhook_list_event_roundtrip() {
    let event = ChatEvent::WebhookList {
        server_id: "srv1".into(),
        webhooks: vec![WebhookInfo {
            id: "wh1".into(),
            server_id: "srv1".into(),
            channel_id: "ch1".into(),
            name: "My Webhook".into(),
            avatar_url: None,
            webhook_type: "incoming".into(),
            token: "token123".into(),
            url: None,
            created_by: "user1".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::WebhookList { webhooks, .. } => {
            assert_eq!(webhooks.len(), 1);
            assert_eq!(webhooks[0].name, "My Webhook");
            assert_eq!(webhooks[0].webhook_type, "incoming");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_slash_command_list_event_roundtrip() {
    let event = ChatEvent::SlashCommandList {
        server_id: "srv1".into(),
        commands: vec![SlashCommandInfo {
            id: "cmd1".into(),
            bot_user_id: "bot1".into(),
            name: "ping".into(),
            description: "Pings the bot".into(),
            options: vec![SlashCommandOption {
                name: "target".into(),
                description: "Who to ping".into(),
                option_type: "user".into(),
                required: true,
                choices: None,
            }],
        }],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::SlashCommandList { commands, .. } => {
            assert_eq!(commands.len(), 1);
            assert_eq!(commands[0].name, "ping");
            assert_eq!(commands[0].options.len(), 1);
            assert!(commands[0].options[0].required);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_oauth2_app_list_event_roundtrip() {
    let event = ChatEvent::OAuth2AppList {
        apps: vec![OAuth2AppInfo {
            id: "app1".into(),
            name: "My App".into(),
            description: "Test app".into(),
            icon_url: None,
            owner_id: "user1".into(),
            redirect_uris: vec!["https://example.com/callback".into()],
            scopes: "identify".into(),
            is_public: true,
            created_at: "2026-01-01T00:00:00Z".into(),
        }],
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::OAuth2AppList { apps } => {
            assert_eq!(apps.len(), 1);
            assert_eq!(apps[0].name, "My App");
            assert!(apps[0].is_public);
            assert_eq!(apps[0].redirect_uris.len(), 1);
        }
        _ => panic!("Wrong variant"),
    }
}
