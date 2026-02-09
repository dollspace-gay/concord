/// Server-level roles ordered by privilege level.
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
}
