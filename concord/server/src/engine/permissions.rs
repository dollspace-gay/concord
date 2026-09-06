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
mod tests;
