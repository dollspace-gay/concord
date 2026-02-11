use bitflags::bitflags;

bitflags! {
    /// Permission bitfield for roles and channel overrides.
    /// Stored as `i64` in SQLite (cast to/from `u64`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Permissions: u64 {
        // ── General ──
        const VIEW_CHANNELS         = 1 << 0;
        const MANAGE_CHANNELS       = 1 << 1;
        const MANAGE_ROLES          = 1 << 2;
        const MANAGE_SERVER         = 1 << 3;
        const CREATE_INVITES        = 1 << 4;
        const KICK_MEMBERS          = 1 << 5;
        const BAN_MEMBERS           = 1 << 6;
        const ADMINISTRATOR         = 1 << 7;

        // ── Channel text ──
        const SEND_MESSAGES         = 1 << 10;
        const EMBED_LINKS           = 1 << 11;
        const ATTACH_FILES          = 1 << 12;
        const ADD_REACTIONS         = 1 << 13;
        const MENTION_EVERYONE      = 1 << 14;
        const MANAGE_MESSAGES       = 1 << 15;
        const READ_MESSAGE_HISTORY  = 1 << 16;

        // ── Voice (future) ──
        const CONNECT               = 1 << 20;
        const SPEAK                 = 1 << 21;
        const MUTE_MEMBERS          = 1 << 22;
        const DEAFEN_MEMBERS        = 1 << 23;
        const MOVE_MEMBERS          = 1 << 24;
    }
}

/// Default permissions for the @everyone role.
pub const DEFAULT_EVERYONE: Permissions = Permissions::VIEW_CHANNELS
    .union(Permissions::SEND_MESSAGES)
    .union(Permissions::EMBED_LINKS)
    .union(Permissions::ATTACH_FILES)
    .union(Permissions::ADD_REACTIONS)
    .union(Permissions::READ_MESSAGE_HISTORY)
    .union(Permissions::CREATE_INVITES);

/// Default permissions for a Moderator role.
pub const DEFAULT_MODERATOR: Permissions = DEFAULT_EVERYONE
    .union(Permissions::KICK_MEMBERS)
    .union(Permissions::MANAGE_MESSAGES)
    .union(Permissions::MENTION_EVERYONE);

/// Default permissions for an Admin role.
pub const DEFAULT_ADMIN: Permissions = DEFAULT_MODERATOR
    .union(Permissions::MANAGE_CHANNELS)
    .union(Permissions::MANAGE_ROLES)
    .union(Permissions::MANAGE_SERVER)
    .union(Permissions::BAN_MEMBERS);

/// A channel permission override (allow/deny pair).
#[derive(Debug, Clone)]
pub struct ChannelOverride {
    pub target_type: OverrideTargetType,
    pub target_id: String,
    pub allow: Permissions,
    pub deny: Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideTargetType {
    Role,
    User,
}

/// Compute a user's effective permissions in a channel.
///
/// Algorithm (mirrors Discord):
///   1. Server owner gets all permissions unconditionally.
///   2. Start with `@everyone` role's base permissions.
///   3. OR in all the user's assigned role permissions.
///   4. If ADMINISTRATOR is set, return all permissions.
///   5. Apply channel overrides for `@everyone` role (allow OR, deny AND NOT).
///   6. For each of the user's roles, collect allow/deny from overrides.
///   7. OR all role allows, AND NOT all role denies.
///   8. Apply user-specific override (allow OR, deny AND NOT).
pub fn compute_effective_permissions(
    base_everyone: Permissions,
    user_role_permissions: &[(String, Permissions)],
    channel_overrides: &[ChannelOverride],
    everyone_role_id: &str,
    user_id: &str,
    is_owner: bool,
) -> Permissions {
    if is_owner {
        return Permissions::all();
    }

    // Step 1-2: base = @everyone perms | all user role perms
    let mut perms = base_everyone;
    for (_role_id, role_perms) in user_role_permissions {
        perms |= *role_perms;
    }

    // Step 3: admin bypass
    if perms.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all();
    }

    // If no channel overrides, we're done (server-level permissions)
    if channel_overrides.is_empty() {
        return perms;
    }

    // Step 4: apply @everyone channel override
    for ov in channel_overrides {
        if ov.target_type == OverrideTargetType::Role && ov.target_id == everyone_role_id {
            perms |= ov.allow;
            perms &= !ov.deny;
        }
    }

    // Step 5-6: collect role allows/denies
    let user_role_ids: Vec<&str> = user_role_permissions
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    let mut role_allow = Permissions::empty();
    let mut role_deny = Permissions::empty();
    for ov in channel_overrides {
        if ov.target_type == OverrideTargetType::Role
            && ov.target_id != everyone_role_id
            && user_role_ids.contains(&ov.target_id.as_str())
        {
            role_allow |= ov.allow;
            role_deny |= ov.deny;
        }
    }
    perms |= role_allow;
    perms &= !role_deny;

    // Step 7: apply user-specific override
    for ov in channel_overrides {
        if ov.target_type == OverrideTargetType::User && ov.target_id == user_id {
            perms |= ov.allow;
            perms &= !ov.deny;
        }
    }

    perms
}

// ── Legacy role compat ──────────────────────────────────────

/// Server-level roles ordered by privilege level (legacy, kept for backward compat).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServerRole {
    Member,
    Moderator,
    Admin,
    Owner,
}

impl ServerRole {
    pub fn parse(s: &str) -> Self {
        match s {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            "moderator" => Self::Moderator,
            _ => Self::Member,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Moderator => "moderator",
            Self::Member => "member",
        }
    }

    /// Map this legacy role to a default permission bitfield.
    pub fn to_default_permissions(&self) -> Permissions {
        match self {
            Self::Member => DEFAULT_EVERYONE,
            Self::Moderator => DEFAULT_MODERATOR,
            Self::Admin => DEFAULT_ADMIN,
            Self::Owner => Permissions::all(),
        }
    }

    pub fn can_manage_channels(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    pub fn can_kick_members(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Moderator)
    }

    pub fn can_delete_messages(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Moderator)
    }

    pub fn can_manage_roles(&self, target: &ServerRole) -> bool {
        self > target
    }

    pub fn can_delete_server(&self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn can_update_server(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

#[cfg(test)]
mod tests {
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

    // ────────────────────────────────────────────────────────────────
    // Additional edge case tests for compute_effective_permissions
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_empty_role_list_just_everyone_base() {
        // User with no assigned roles — only @everyone base applies
        let perms = compute_effective_permissions(
            Permissions::VIEW_CHANNELS | Permissions::SEND_MESSAGES,
            &[],
            &[],
            "everyone-id",
            "user1",
            false,
        );
        assert!(perms.contains(Permissions::VIEW_CHANNELS));
        assert!(perms.contains(Permissions::SEND_MESSAGES));
        assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
        assert!(!perms.contains(Permissions::ADMINISTRATOR));
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

    // ────────────────────────────────────────────────────────────────
    // ServerRole additional tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_role_as_str_roundtrip() {
        for role in [
            ServerRole::Owner,
            ServerRole::Admin,
            ServerRole::Moderator,
            ServerRole::Member,
        ] {
            assert_eq!(ServerRole::parse(role.as_str()), role);
        }
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
    fn test_role_delete_messages() {
        assert!(ServerRole::Owner.can_delete_messages());
        assert!(ServerRole::Admin.can_delete_messages());
        assert!(ServerRole::Moderator.can_delete_messages());
        assert!(!ServerRole::Member.can_delete_messages());
    }

    #[test]
    fn test_role_update_server() {
        assert!(ServerRole::Owner.can_update_server());
        assert!(ServerRole::Admin.can_update_server());
        assert!(!ServerRole::Moderator.can_update_server());
        assert!(!ServerRole::Member.can_update_server());
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

    #[test]
    fn test_override_target_type_equality() {
        assert_eq!(OverrideTargetType::Role, OverrideTargetType::Role);
        assert_eq!(OverrideTargetType::User, OverrideTargetType::User);
        assert_ne!(OverrideTargetType::Role, OverrideTargetType::User);
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
}
