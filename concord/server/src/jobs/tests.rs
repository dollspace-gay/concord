use super::*;
use crate::db::pool::{create_pool, run_migrations};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingDispatcher(AtomicUsize);
impl JobDispatcher for CountingDispatcher {
    fn dispatch<'a>(
        &'a self,
        job: &'a ClaimedJob,
    ) -> Pin<Box<dyn Future<Output = Result<(), DispatchFailure>> + Send + 'a>> {
        Box::pin(async move {
            let _: serde_json::Value = serde_json::from_str(&job.payload_json).unwrap();
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
#[tokio::test]
async fn deduplicates_leases_and_retries_safely() {
    let p = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&p).await.unwrap();
    let j = EnqueueJob {
        deduplication_key: "event:1:webhook:2",
        operation_type: "webhook",
        resource_id: "event:1",
        resource_version: 1,
        destination_grant: "webhook:2",
        payload: &serde_json::json!({"event_id":"1"}),
    };
    let first = enqueue(&p, j).await.unwrap();
    let second = enqueue(
        &p,
        EnqueueJob {
            deduplication_key: "event:1:webhook:2",
            operation_type: "webhook",
            resource_id: "event:1",
            resource_version: 1,
            destination_grant: "webhook:2",
            payload: &serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    assert_eq!(first, second);
    let claimed = claim(&p, "worker-a", 30, 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert!(claim(&p, "worker-b", 30, 10).await.unwrap().is_empty());
    assert!(
        fail(
            &p,
            &FailJob {
                id: &first,
                worker: "worker-a",
                lease_token: &claimed[0].lease_token,
                error_code: "timeout",
                retry_after_seconds: Some(0),
                max_attempts: 3,
                permanent: false,
            },
        )
        .await
        .unwrap()
    );
    let retried = claim(&p, "worker-b", 30, 10).await.unwrap();
    assert_eq!(retried[0].attempt_count, 2);
    assert!(
        complete(&p, &first, "worker-b", &retried[0].lease_token)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn stale_attempt_is_fenced_after_same_worker_reclaims() {
    let p = create_pool("sqlite::memory:").await.unwrap();
    run_migrations(&p).await.unwrap();
    let id = enqueue(
        &p,
        EnqueueJob {
            deduplication_key: "resource:1:publish",
            operation_type: "publish",
            resource_id: "resource:1",
            resource_version: 1,
            destination_grant: "grant:1",
            payload: &serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    let first = claim(&p, "stable-worker-id", 30, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    sqlx::query("UPDATE external_jobs SET lease_until='2000-01-01 00:00:00' WHERE id=?")
        .bind(&id)
        .execute(&p)
        .await
        .unwrap();
    let second = claim(&p, "stable-worker-id", 30, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_ne!(first.lease_token, second.lease_token);
    assert!(
        !complete(&p, &id, "stable-worker-id", &first.lease_token)
            .await
            .unwrap()
    );
    assert!(
        !fail(
            &p,
            &FailJob {
                id: &id,
                worker: "stable-worker-id",
                lease_token: &first.lease_token,
                error_code: "stale",
                retry_after_seconds: None,
                max_attempts: 3,
                permanent: false,
            },
        )
        .await
        .unwrap()
    );
    assert!(
        complete(&p, &id, "stable-worker-id", &second.lease_token)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn replacement_worker_dispatches_an_expired_lease_after_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!("sqlite://{}", directory.path().join("jobs.db").display());
    let p = create_pool(&database_url).await.unwrap();
    run_migrations(&p).await.unwrap();
    let id = enqueue(
        &p,
        EnqueueJob {
            deduplication_key: "resource:2:publish",
            operation_type: "publish",
            resource_id: "resource:2",
            resource_version: 1,
            destination_grant: "grant:2",
            payload: &serde_json::json!({"record":"durable"}),
        },
    )
    .await
    .unwrap();
    assert_eq!(claim(&p, "crashed-process", 30, 1).await.unwrap().len(), 1);
    sqlx::query("UPDATE external_jobs SET lease_until='2000-01-01 00:00:00' WHERE id=?")
        .bind(&id)
        .execute(&p)
        .await
        .unwrap();
    p.close().await;

    let reopened = create_pool(&database_url).await.unwrap();
    let dispatcher = CountingDispatcher(AtomicUsize::new(0));
    let report = run_once(&reopened, "replacement-process", &dispatcher, 30, 10, 3)
        .await
        .unwrap();
    assert_eq!(
        report,
        WorkerReport {
            claimed: 1,
            succeeded: 1,
            retried_or_failed: 0,
            lease_lost: 0
        }
    );
    assert_eq!(dispatcher.0.load(Ordering::SeqCst), 1);
    let state: String = sqlx::query_scalar("SELECT state FROM external_jobs WHERE id=?")
        .bind(id)
        .fetch_one(&reopened)
        .await
        .unwrap();
    assert_eq!(state, "succeeded");
}
