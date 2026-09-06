use super::*;

#[tokio::test]
async fn unresolved_migration_014_user_override_remains_denied_and_reported() {
    let pool = historical_fixture(14).await;
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','legacy-user-14')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#safe')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id) VALUES('override','channel','user','old-uuid')").execute(&pool).await.unwrap();
    let error = run_migrations(&pool).await.unwrap_err().to_string();
    assert!(
        error.contains("audited identity mapping required"),
        "{error}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "old-uuid"
    );
    assert!(
        !object_exists(
            &mut pool.acquire().await.unwrap(),
            "table",
            "migration_metadata"
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn operator_user_override_repair_preserves_evidence_and_permission_bits() {
    let pool = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('mapped','mapped')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
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
         ) VALUES('override','channel','user','legacy-uuid',17,4)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let inventory = migration_preflight(&pool).await.unwrap();
    assert!(inventory.findings.iter().any(|finding| {
        finding.code == "unresolved_user_override" && finding.object_id == "override"
    }));
    let repair = repair_user_override(
        &pool,
        "override",
        "mapped",
        "operator ticket MIG-42 verified account ownership",
    )
    .await
    .unwrap();
    assert_eq!(repair.previous_target_id, "legacy-uuid");
    assert_eq!((repair.allow_bits, repair.deny_bits), (17, 4));
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT target_id FROM channel_permission_overrides WHERE id='override'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "mapped"
    );
    let evidence: String = sqlx::query_scalar(
        "SELECT details FROM migration_repair_log \
         WHERE repair_kind='post014_user_override' AND object_id='override'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(evidence.contains("legacy-uuid"));
    assert!(evidence.contains("MIG-42"));
    assert!(!migration_preflight(&pool).await.unwrap().is_blocked());
}
