use super::*;

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
