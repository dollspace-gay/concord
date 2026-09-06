use super::*;

#[test]
fn test_role_ordering() {
    assert!(ServerRole::Owner > ServerRole::Admin);
    assert!(ServerRole::Admin > ServerRole::Moderator);
    assert!(ServerRole::Moderator > ServerRole::Member);
}

#[test]
fn test_role_from_str() {
    assert_eq!(ServerRole::parse("owner"), ServerRole::Owner);
    assert_eq!(ServerRole::parse("admin"), ServerRole::Admin);
    assert_eq!(ServerRole::parse("moderator"), ServerRole::Moderator);
    assert_eq!(ServerRole::parse("member"), ServerRole::Member);
    assert_eq!(ServerRole::parse("unknown"), ServerRole::Member);
}

#[test]
fn test_bitfield_operations() {
    let perms = Permissions::VIEW_CHANNELS | Permissions::SEND_MESSAGES;
    assert!(perms.contains(Permissions::VIEW_CHANNELS));
    assert!(perms.contains(Permissions::SEND_MESSAGES));
    assert!(!perms.contains(Permissions::MANAGE_CHANNELS));

    let combined = perms | Permissions::MANAGE_CHANNELS;
    assert!(combined.contains(Permissions::MANAGE_CHANNELS));

    let denied = combined & !Permissions::SEND_MESSAGES;
    assert!(!denied.contains(Permissions::SEND_MESSAGES));
    assert!(denied.contains(Permissions::VIEW_CHANNELS));
}

#[test]
fn test_role_override_re_allows_after_everyone_deny() {
    // @everyone override denies SEND_MESSAGES, but user's role override re-allows it
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[("mod-role".to_string(), Permissions::KICK_MEMBERS)],
        &[
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "everyone-id".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            },
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "mod-role".to_string(),
                allow: Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
            },
        ],
        "everyone-id",
        "user1",
        false,
    );
    // Role override re-allows SEND_MESSAGES
    assert!(perms.contains(Permissions::SEND_MESSAGES));
}

#[test]
fn test_user_override_takes_precedence_over_role_override() {
    // Role override allows SEND_MESSAGES, but user override denies it
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[("mod-role".to_string(), Permissions::KICK_MEMBERS)],
        &[
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "mod-role".to_string(),
                allow: Permissions::MANAGE_CHANNELS,
                deny: Permissions::empty(),
            },
            ChannelOverride {
                target_type: OverrideTargetType::User,
                target_id: "user1".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::MANAGE_CHANNELS | Permissions::SEND_MESSAGES,
            },
        ],
        "everyone-id",
        "user1",
        false,
    );
    // User override denies both
    assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
    assert!(!perms.contains(Permissions::SEND_MESSAGES));
    // But KICK_MEMBERS from role remains
    assert!(perms.contains(Permissions::KICK_MEMBERS));
}

#[test]
fn test_administrator_bypasses_all_deny_bits() {
    // Even if there are channel deny overrides, ADMINISTRATOR bypasses everything
    let perms = compute_effective_permissions(
        Permissions::VIEW_CHANNELS,
        &[("admin-role".to_string(), Permissions::ADMINISTRATOR)],
        &[
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "everyone-id".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::all(),
            },
            ChannelOverride {
                target_type: OverrideTargetType::User,
                target_id: "user1".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::all(),
            },
        ],
        "everyone-id",
        "user1",
        false,
    );
    assert_eq!(perms, Permissions::all());
}

#[test]
fn test_override_for_unrelated_role_is_ignored() {
    // Override for a role the user doesn't have should be ignored
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[],
        &[ChannelOverride {
            target_type: OverrideTargetType::Role,
            target_id: "other-role".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        }],
        "everyone-id",
        "user1",
        false,
    );
    // The deny on other-role should not affect user1
    assert!(perms.contains(Permissions::SEND_MESSAGES));
}

#[test]
fn test_override_for_unrelated_user_is_ignored() {
    // Override for a different user should be ignored
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[],
        &[ChannelOverride {
            target_type: OverrideTargetType::User,
            target_id: "other-user".to_string(),
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        }],
        "everyone-id",
        "user1",
        false,
    );
    assert!(perms.contains(Permissions::SEND_MESSAGES));
}

#[test]
fn test_multiple_role_overrides_are_combined() {
    // User has two roles with channel overrides — allows and denies are collected
    let perms = compute_effective_permissions(
        DEFAULT_EVERYONE,
        &[
            ("role-a".to_string(), Permissions::empty()),
            ("role-b".to_string(), Permissions::empty()),
        ],
        &[
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "role-a".to_string(),
                allow: Permissions::MANAGE_CHANNELS,
                deny: Permissions::empty(),
            },
            ChannelOverride {
                target_type: OverrideTargetType::Role,
                target_id: "role-b".to_string(),
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            },
        ],
        "everyone-id",
        "user1",
        false,
    );
    // role-a allows MANAGE_CHANNELS, role-b denies SEND_MESSAGES
    assert!(perms.contains(Permissions::MANAGE_CHANNELS));
    assert!(!perms.contains(Permissions::SEND_MESSAGES));
}

#[test]
fn test_everyone_base_empty_gives_no_perms() {
    let perms = compute_effective_permissions(
        Permissions::empty(),
        &[],
        &[],
        "everyone-id",
        "user1",
        false,
    );
    assert_eq!(perms, Permissions::empty());
}

#[test]
fn test_role_can_manage_roles_hierarchy() {
    // Owner can manage all
    assert!(ServerRole::Owner.can_manage_roles(&ServerRole::Admin));
    assert!(ServerRole::Owner.can_manage_roles(&ServerRole::Moderator));
    assert!(ServerRole::Owner.can_manage_roles(&ServerRole::Member));
    // Admin can manage mod and member
    assert!(ServerRole::Admin.can_manage_roles(&ServerRole::Moderator));
    assert!(ServerRole::Admin.can_manage_roles(&ServerRole::Member));
    // Admin cannot manage owner or self
    assert!(!ServerRole::Admin.can_manage_roles(&ServerRole::Owner));
    assert!(!ServerRole::Admin.can_manage_roles(&ServerRole::Admin));
    // Mod can manage member
    assert!(ServerRole::Moderator.can_manage_roles(&ServerRole::Member));
    // Mod cannot manage self or above
    assert!(!ServerRole::Moderator.can_manage_roles(&ServerRole::Moderator));
    // Member cannot manage anyone
    assert!(!ServerRole::Member.can_manage_roles(&ServerRole::Member));
}

#[test]
fn test_user_override_allow_and_deny_same_bit() {
    // When user override both allows and denies the same bit, deny wins
    // (applied as: perms |= allow; perms &= !deny; — deny is applied last)
    let perms = compute_effective_permissions(
        Permissions::VIEW_CHANNELS,
        &[],
        &[ChannelOverride {
            target_type: OverrideTargetType::User,
            target_id: "user1".to_string(),
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::SEND_MESSAGES,
        }],
        "everyone-id",
        "user1",
        false,
    );
    // deny is applied after allow, so SEND_MESSAGES should be denied
    assert!(!perms.contains(Permissions::SEND_MESSAGES));
}
