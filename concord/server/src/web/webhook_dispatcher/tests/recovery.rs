use super::*;

#[tokio::test]
async fn retryable_http_failure_becomes_terminal_at_attempt_limit() {
    assert_eq!(
        run_http_failure(503, 7).await,
        (
            "failed".into(),
            "failed".into(),
            8,
            "webhook_http_retryable".into()
        )
    );
}
