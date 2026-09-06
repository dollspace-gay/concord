use super::*;

#[tokio::test]
async fn deleted_source_is_rejected_without_transmitting_queued_content() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (pool, vault, transport) = queued_delivery_fixture(address).await;
    sqlx::query("UPDATE messages SET deleted_at=datetime('now') WHERE id='message'")
        .execute(&pool)
        .await
        .unwrap();
    let dispatcher = WebhookDispatcher::new(pool.clone(), transport, vault, 8);
    let report = crate::jobs::run_once_matching(
        &pool,
        "worker",
        &dispatcher,
        &crate::jobs::JobSelection {
            operation_types: &["webhook_delivery"],
            lease_seconds: 30,
            limit: 1,
            max_attempts: 8,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.retried_or_failed, 1);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
    let state: (String, String) = sqlx::query_as("SELECT d.state,j.state FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("failed".into(), "failed".into()));
}
