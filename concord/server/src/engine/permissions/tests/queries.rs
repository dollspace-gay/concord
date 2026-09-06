use super::*;

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
fn test_override_target_type_equality() {
    assert_eq!(OverrideTargetType::Role, OverrideTargetType::Role);
    assert_eq!(OverrideTargetType::User, OverrideTargetType::User);
    assert_ne!(OverrideTargetType::Role, OverrideTargetType::User);
}
