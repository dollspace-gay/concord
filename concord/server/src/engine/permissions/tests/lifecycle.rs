use super::*;

#[test]
fn test_role_update_server() {
    assert!(ServerRole::Owner.can_update_server());
    assert!(ServerRole::Admin.can_update_server());
    assert!(!ServerRole::Moderator.can_update_server());
    assert!(!ServerRole::Member.can_update_server());
}
