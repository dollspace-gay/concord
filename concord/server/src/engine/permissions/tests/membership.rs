use super::*;

#[test]
fn test_channel_override_deny_overrides_role_allow() {
    // Role gives SEND_MESSAGES, but channel override denies it for @everyone
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[("mod-role".to_string(), Permissions::KICK_MEMBERS)],
        &[ChannelOverride {
            target_type: OverrideTargetType::Role,
            target_id: "everyone-id".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        }],
        "everyone-id",
        "user1",
        false,
    );
    // SEND_MESSAGES denied by @everyone override
    assert!(!perms.contains(Permissions::SEND_MESSAGES));
    // But other perms remain
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(perms.contains(Permissions::KICK_MEMBERS));
}

#[test]
fn test_no_channel_overrides_returns_server_level_perms() {
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[(
            "mod-role".to_string(),
            Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES,
        )],
        &[], // no channel overrides
        "everyone-id",
        "user1",
        false,
    );
    // Should just be everyone | mod perms
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(perms.contains(Permissions::SEND_MESSAGES));
    assert!(perms.contains(Permissions::KICK_MEMBERS));
    assert!(perms.contains(Permissions::MANAGE_MESSAGES));
}

#[test]
fn test_channel_override_debug() {
    // Verify ChannelOverride implements Debug
    let ov = ChannelOverride {
        target_type: OverrideTargetType::Role,
        target_id: "test".to_string(),
        allow: Permissions::empty(),
        deny: Permissions::empty(),
    };
    let debug = format!("{:?}", ov);
    assert!(debug.contains("Role"));
}
