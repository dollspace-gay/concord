use super::*;

#[tokio::test]
async fn test_invite_with_expiry() {
    let pool = setup_db().await;

    let owner_id = create_test_user(&pool, "alice").await;

    let server_id = "expiry-server";
    queries::servers::create_server(&pool, server_id, "Expiry Test", &owner_id, None)
        .await
        .unwrap();

    // Create an already-expired invite
    let invite_id = Uuid::new_v4().to_string();
    queries::invites::create_invite(
        &pool,
        &invite_id,
        server_id,
        "EXPIRED1",
        &owner_id,
        None,
        Some("2020-01-01T00:00:00Z"), // already expired
        None,
    )
    .await
    .unwrap();

    // Create a future invite
    let invite_id2 = Uuid::new_v4().to_string();
    queries::invites::create_invite(
        &pool,
        &invite_id2,
        server_id,
        "FUTURE1",
        &owner_id,
        None,
        Some("2099-12-31T23:59:59Z"),
        None,
    )
    .await
    .unwrap();

    // Delete expired invites
    let deleted = queries::invites::delete_expired_invites(&pool)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "One expired invite should be deleted");

    // Verify the future invite still exists
    let future = queries::invites::get_invite_by_code(&pool, "FUTURE1")
        .await
        .unwrap();
    assert!(future.is_some());

    // Verify the expired invite is gone
    let expired = queries::invites::get_invite_by_code(&pool, "EXPIRED1")
        .await
        .unwrap();
    assert!(expired.is_none());
}
