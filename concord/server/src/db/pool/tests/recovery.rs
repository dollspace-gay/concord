use super::*;

#[tokio::test]
async fn populated_default_orphan_returns_repair_report_without_mutation() {
    let pool = historical_fixture(2).await;
    let mut fixture_conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *fixture_conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('legacy-message','default','#general','legacy-user-2','legacy2','preserve me')")
        .execute(&mut *fixture_conn).await.unwrap();
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *fixture_conn)
        .await
        .unwrap();
    drop(fixture_conn);
    let error = run_migrations(&pool).await.unwrap_err().to_string();
    assert!(
        error.contains("operator server mapping required"),
        "{error}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE id='legacy-message'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
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
