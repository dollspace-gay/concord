use super::*;

#[test]
fn test_role_delete_messages() {
    assert!(ServerRole::Owner.can_delete_messages());
    assert!(ServerRole::Admin.can_delete_messages());
    assert!(ServerRole::Moderator.can_delete_messages());
    assert!(!ServerRole::Member.can_delete_messages());
}
