use super::*;

#[tokio::test]
async fn fresh_database_has_verified_history_and_durable_profile() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_version ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(versions, (1..=current_schema_version()).collect::<Vec<_>>());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM migration_metadata")
            .fetch_one(&pool)
            .await
            .unwrap(),
        current_schema_version()
    );
    let profile = (
        sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
            .fetch_one(&pool)
            .await
            .unwrap(),
    );
    assert_eq!(profile, (1, 2));
    let snapshots_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM migration_snapshots")
        .fetch_one(&pool)
        .await
        .unwrap();
    run_migrations(&pool).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let snapshots_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM migration_snapshots")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(snapshots_after, snapshots_before);
}

#[tokio::test]
async fn ambiguous_pre014_identity_and_override_collision_are_not_guessed() {
    let pool = historical_fixture(13).await;
    sqlx::query(
        "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES \
         ('at-one','legacy-user-13','atproto','did:plc:one'), \
         ('at-two','legacy-user-13','atproto','did:plc:two')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO servers(id,name,owner_id) \
         VALUES('server','Server','legacy-user-13')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#safe')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO channel_permission_overrides( \
            id,channel_id,target_type,target_id,allow_bits,deny_bits \
         ) VALUES \
            ('legacy-override','channel','user','legacy-user-13',5,2), \
            ('current-override','channel','user','did:plc:one',8,1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let report = migration_preflight(&pool).await.unwrap();
    assert!(report.findings.iter().any(|finding| {
        finding.code == "ambiguous_pre014_at_identity" && finding.object_id == "legacy-user-13"
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.code == "pre014_override_target_collision" && finding.object_id == "legacy-override"
    }));
    assert!(run_migrations(&pool).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT target_id FROM channel_permission_overrides \
             WHERE id='legacy-override'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "legacy-user-13"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        13
    );
}

#[tokio::test]
async fn audit_actor_snapshot_survives_actor_account_deletion() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users(id,username,avatar_url) VALUES \
         ('owner','owner',NULL),('moderator','mod-at-action','avatar-at-action')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
        .execute(&pool)
        .await
        .unwrap();
    crate::db::queries::audit_log::create_entry(
        &pool,
        &crate::db::models::CreateAuditLogParams {
            id: "audit",
            server_id: "server",
            actor_id: "moderator",
            action_type: "member_kick",
            target_type: Some("user"),
            target_id: Some("target"),
            reason: None,
            changes: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM users WHERE id='moderator'")
        .execute(&pool)
        .await
        .unwrap();
    let snapshot: (String, String, Option<String>) = sqlx::query_as(
        "SELECT actor_id,actor_username_snapshot,actor_avatar_snapshot \
         FROM audit_log WHERE id='audit'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        snapshot,
        (
            "moderator".into(),
            "mod-at-action".into(),
            Some("avatar-at-action".into())
        )
    );
}
