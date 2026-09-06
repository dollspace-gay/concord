use super::*;

#[tokio::test]
async fn expired_generation_rejects_an_operation_without_a_retained_receipt() {
    let (pool, _, actor, service) = fixture().await;
    let generation: String = sqlx::query_scalar(
        "SELECT current_generation FROM operation_generation_state WHERE singleton=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE operation_generations SET issued_at=unixepoch()-2,expires_at=unixepoch()-1 \
         WHERE generation=?",
    )
    .bind(&generation)
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        service
            .send_channel_message(
                &actor,
                command_in_generation("request", "missing-client", "hello", &generation),
            )
            .await,
        Err(MessagingError::OperationGenerationExpired)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
