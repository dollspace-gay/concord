use super::*;

#[test]
fn test_nick_change_event_roundtrip() {
    let event = ChatEvent::NickChange {
        old_nick: "alice".into(),
        new_nick: "alice2".into(),
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::NickChange { old_nick, new_nick } => {
            assert_eq!(old_nick, "alice");
            assert_eq!(new_nick, "alice2");
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_bluesky_profile_sync_roundtrip() {
    let event = ChatEvent::BlueskyProfileSync {
        user_id: "user1".into(),
        bsky_handle: "alice.bsky.social".into(),
        display_name: Some("Alice".into()),
        description: Some("Hello world".into()),
        avatar_url: Some("https://cdn.bsky.app/avatar.jpg".into()),
        banner_url: None,
        followers_count: 150,
        follows_count: 42,
    };
    let restored = roundtrip(&event);
    match restored {
        ChatEvent::BlueskyProfileSync {
            user_id,
            bsky_handle,
            display_name,
            followers_count,
            ..
        } => {
            assert_eq!(user_id, "user1");
            assert_eq!(bsky_handle, "alice.bsky.social");
            assert_eq!(display_name.as_deref(), Some("Alice"));
            assert_eq!(followers_count, 150);
        }
        _ => panic!("Wrong variant"),
    }
}
