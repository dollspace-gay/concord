use super::*;

#[test]
fn test_permissions() {
    assert!(ServerRole::Owner.can_manage_channels());
    assert!(ServerRole::Admin.can_manage_channels());
    assert!(!ServerRole::Moderator.can_manage_channels());
    assert!(!ServerRole::Member.can_manage_channels());

    assert!(ServerRole::Moderator.can_kick_members());
    assert!(!ServerRole::Member.can_kick_members());

    assert!(ServerRole::Owner.can_delete_server());
    assert!(!ServerRole::Admin.can_delete_server());

    assert!(ServerRole::Admin.can_manage_roles(&ServerRole::Moderator));
    assert!(!ServerRole::Moderator.can_manage_roles(&ServerRole::Admin));
}

#[test]
fn test_default_permissions() {
    assert!(DEFAULT_EVERYONE.contains(Permissions::VIEW_CHANNELS));
    assert!(DEFAULT_EVERYONE.contains(Permissions::SEND_MESSAGES));
    assert!(!DEFAULT_EVERYONE.contains(Permissions::MANAGE_CHANNELS));
    assert!(!DEFAULT_EVERYONE.contains(Permissions::ADMINISTRATOR));

    assert!(DEFAULT_MODERATOR.contains(Permissions::KICK_MEMBERS));
    assert!(DEFAULT_MODERATOR.contains(Permissions::MANAGE_MESSAGES));
    assert!(!DEFAULT_MODERATOR.contains(Permissions::MANAGE_CHANNELS));

    assert!(DEFAULT_ADMIN.contains(Permissions::MANAGE_CHANNELS));
    assert!(DEFAULT_ADMIN.contains(Permissions::MANAGE_ROLES));
    assert!(!DEFAULT_ADMIN.contains(Permissions::ADMINISTRATOR));
}

#[test]
fn test_legacy_role_to_permissions() {
    assert_eq!(
        ServerRole::Member.to_default_permissions(),
        DEFAULT_EVERYONE
    );
    assert_eq!(
        ServerRole::Moderator.to_default_permissions(),
        DEFAULT_MODERATOR
    );
    assert_eq!(ServerRole::Admin.to_default_permissions(), DEFAULT_ADMIN);
    assert_eq!(
        ServerRole::Owner.to_default_permissions(),
        Permissions::all()
    );
}

#[test]
fn test_effective_permissions_basic() {
    // User with only @everyone role
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[],
        &[],
        "everyone-role-id",
        "user1",
        false,
    );
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(perms.contains(Permissions::SEND_MESSAGES));
    assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
}

#[test]
fn test_effective_permissions_multi_role() {
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[("mod-role".to_string(), DEFAULT_MODERATOR)],
        &[],
        "everyone-role-id",
        "user1",
        false,
    );
    assert!(perms.contains(Permissions::KICK_MEMBERS));
    assert!(perms.contains(Permissions::MANAGE_MESSAGES));
    assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
}

#[test]
fn test_effective_permissions_admin_bypass() {
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[("admin-role".to_string(), Permissions::ADMINISTRATOR)],
        &[ChannelOverride {
            target_type: OverrideTargetType::User,
            target_id: "user1".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        }],
        "everyone-role-id",
        "user1",
        false,
    );
    // ADMINISTRATOR bypasses all — even explicit denies are ignored
    assert!(perms.contains(Permissions::SEND_MESSAGES));
    assert_eq!(perms, Permissions::all());
}

#[test]
fn test_effective_permissions_owner_bypass() {
    let perms = compute_effective_permissions(
        Permissions::empty(),
        &[],
        &[ChannelOverride {
            target_type: OverrideTargetType::User,
            target_id: "owner1".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::all(),
        }],
        "everyone-role-id",
        "owner1",
        true, // is_owner
    );
    assert_eq!(perms, Permissions::all());
}

#[test]
fn test_effective_permissions_channel_override_deny() {
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[],
        &[
            // Deny SEND_MESSAGES for @everyone in this channel
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "everyone-role-id".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            },
        ],
        "everyone-role-id",
        "user1",
        false,
    );
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(!perms.contains(Permissions::SEND_MESSAGES));
}

#[test]
fn test_effective_permissions_user_override() {
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[],
        &[
            // Deny everyone from sending
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "everyone-role-id".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            },
            // But allow this specific user
            ChannelOverride {
                target_type: OverrideTargetType::User,
                target_id: "special-user".to_string(),
                allow: Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
            },
        ],
        "everyone-role-id",
        "special-user",
        false,
    );
    assert!(perms.contains(Permissions::SEND_MESSAGES));
}

#[test]
fn test_multiple_roles_permissions_are_ored() {
    // Multiple roles should have their permissions OR'd together
    let role1_perms = Permissions::KICK_MEMBERS;
    let role2_perms = Permissions::BAN_MEMBERS;
    let perms = compute_effective_permissions(
        Permissions::VIEW_CHANNELS,
        &[
            ("role1".to_string(), role1_perms),
            ("role2".to_string(), role2_perms),
        ],
        &[],
        "everyone-id",
        "user1",
        false,
    );
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(perms.contains(Permissions::KICK_MEMBERS));
    assert!(perms.contains(Permissions::BAN_MEMBERS));
}

#[test]
fn test_owner_gets_all_permissions_regardless_of_everything() {
    // Server owner always gets all permissions, regardless of roles and overrides
    let perms = compute_effective_permissions(
        Permissions::empty(),
        &[],
        &[ChannelOverride {
            target_type: OverrideTargetType::Role,
            target_id: "everyone-id".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::all(),
        }],
        "everyone-id",
        "owner-user",
        true,
    );
    assert_eq!(perms, Permissions::all());
}

#[test]
fn test_all_permission_bits() {
    // Verify all permission bits can be set
    let all = Permissions::all();
    assert!(all.contains(Permissions::VIEW_CHANNELS));
    assert!(all.contains(Permissions::MANAGE_CHANNELS));
    assert!(all.contains(Permissions::MANAGE_ROLES));
    assert!(all.contains(Permissions::MANAGE_SERVER));
    assert!(all.contains(Permissions::CREATE_INVITES));
    assert!(all.contains(Permissions::KICK_MEMBERS));
    assert!(all.contains(Permissions::BAN_MEMBERS));
    assert!(all.contains(Permissions::ADMINISTRATOR));
    assert!(all.contains(Permissions::SEND_MESSAGES));
    assert!(all.contains(Permissions::EMBED_LINKS));
    assert!(all.contains(Permissions::ATTACH_FILES));
    assert!(all.contains(Permissions::ADD_REACTIONS));
    assert!(all.contains(Permissions::MENTION_EVERYONE));
    assert!(all.contains(Permissions::MANAGE_MESSAGES));
    assert!(all.contains(Permissions::READ_MESSAGE_HISTORY));
    assert!(all.contains(Permissions::CONNECT));
    assert!(all.contains(Permissions::SPEAK));
    assert!(all.contains(Permissions::MUTE_MEMBERS));
    assert!(all.contains(Permissions::DEAFEN_MEMBERS));
    assert!(all.contains(Permissions::MOVE_MEMBERS));
}

#[test]
fn test_permission_bits_from_u64() {
    let bits = Permissions::VIEW_CHANNELS.bits() | Permissions::SEND_MESSAGES.bits();
    let perms = Permissions::from_bits_truncate(bits);
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(perms.contains(Permissions::SEND_MESSAGES));
    assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
}

#[test]
fn test_permission_i64_roundtrip() {
    // Permissions are stored as i64 in SQLite, verify roundtrip
    let original = DEFAULT_ADMIN;
    let as_i64 = original.bits() as i64;
    let restored = Permissions::from_bits_truncate(as_i64 as u64);
    assert_eq!(original, restored);
}
