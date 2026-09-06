use super::*;

#[tokio::test]
async fn permanent_http_rejection_marks_delivery_and_job_failed() {
    assert_eq!(
        run_http_failure(400, 0).await,
        (
            "failed".into(),
            "failed".into(),
            1,
            "webhook_http_rejected".into()
        )
    );
}
