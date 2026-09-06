//! Durable leases and retry state for external side effects.
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use std::{future::Future, pin::Pin};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("external job database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("external job input is invalid")]
    Invalid,
}
pub struct EnqueueJob<'a> {
    pub deduplication_key: &'a str,
    pub operation_type: &'a str,
    pub resource_id: &'a str,
    pub resource_version: i64,
    pub destination_grant: &'a str,
    pub payload: &'a serde_json::Value,
}
#[derive(Debug, sqlx::FromRow)]
pub struct ClaimedJob {
    pub id: String,
    pub lease_owner: String,
    pub lease_token: String,
    pub operation_type: String,
    pub resource_id: String,
    pub resource_version: i64,
    pub destination_grant: String,
    pub payload_json: String,
    pub attempt_count: i64,
}

#[derive(Debug)]
pub struct DispatchFailure {
    pub safe_code: &'static str,
    pub retry_after_seconds: Option<u64>,
    pub permanent: bool,
}

pub struct FailJob<'a> {
    pub id: &'a str,
    pub worker: &'a str,
    pub lease_token: &'a str,
    pub error_code: &'a str,
    pub retry_after_seconds: Option<u64>,
    pub max_attempts: i64,
    pub permanent: bool,
}
pub struct JobSelection<'a> {
    pub operation_types: &'a [&'a str],
    pub lease_seconds: i64,
    pub limit: i64,
    pub max_attempts: i64,
}

pub trait JobDispatcher: Send + Sync {
    fn dispatch<'a>(
        &'a self,
        job: &'a ClaimedJob,
    ) -> Pin<Box<dyn Future<Output = Result<(), DispatchFailure>> + Send + 'a>>;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkerReport {
    pub claimed: usize,
    pub succeeded: usize,
    pub retried_or_failed: usize,
    pub lease_lost: usize,
}

/// Claims and dispatches one bounded batch. Completion is recorded only after
/// the dispatcher returns success, so an interrupted process leaves a lease
/// that a replacement worker can reclaim after expiry.
pub async fn run_once<D: JobDispatcher>(
    pool: &SqlitePool,
    worker: &str,
    dispatcher: &D,
    lease_seconds: i64,
    limit: i64,
    max_attempts: i64,
) -> Result<WorkerReport, JobError> {
    if max_attempts < 1 {
        return Err(JobError::Invalid);
    }
    let jobs = claim(pool, worker, lease_seconds, limit).await?;
    let mut report = WorkerReport {
        claimed: jobs.len(),
        ..WorkerReport::default()
    };
    for job in jobs {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::JobDispatch);
        match dispatcher.dispatch(&job).await {
            Ok(()) => {
                if complete(pool, &job.id, worker, &job.lease_token).await? {
                    report.succeeded += 1;
                    metric.succeed();
                } else {
                    report.lease_lost += 1;
                }
            }
            Err(error) => {
                if fail(
                    pool,
                    &FailJob {
                        id: &job.id,
                        worker,
                        lease_token: &job.lease_token,
                        error_code: error.safe_code,
                        retry_after_seconds: error.retry_after_seconds,
                        max_attempts,
                        permanent: error.permanent,
                    },
                )
                .await?
                {
                    report.retried_or_failed += 1;
                } else {
                    report.lease_lost += 1;
                }
            }
        }
    }
    Ok(report)
}

pub async fn run_once_matching<D: JobDispatcher>(
    pool: &SqlitePool,
    worker: &str,
    dispatcher: &D,
    selection: &JobSelection<'_>,
) -> Result<WorkerReport, JobError> {
    if selection.max_attempts < 1 {
        return Err(JobError::Invalid);
    }
    let jobs = claim_matching(
        pool,
        worker,
        selection.operation_types,
        selection.lease_seconds,
        selection.limit,
    )
    .await?;
    dispatch_claimed(
        pool,
        worker,
        dispatcher,
        jobs,
        selection.lease_seconds,
        selection.max_attempts,
    )
    .await
}

async fn dispatch_claimed<D: JobDispatcher>(
    pool: &SqlitePool,
    worker: &str,
    dispatcher: &D,
    jobs: Vec<ClaimedJob>,
    lease_seconds: i64,
    max_attempts: i64,
) -> Result<WorkerReport, JobError> {
    let mut report = WorkerReport {
        claimed: jobs.len(),
        ..WorkerReport::default()
    };
    for job in jobs {
        if !renew(pool, &job.id, worker, &job.lease_token, lease_seconds).await? {
            report.lease_lost += 1;
            crate::runtime_metrics::record(
                crate::runtime_metrics::Operation::JobDispatch,
                false,
                std::time::Duration::ZERO,
            );
            continue;
        }
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::JobDispatch);
        match dispatcher.dispatch(&job).await {
            Ok(()) => {
                if complete(pool, &job.id, worker, &job.lease_token).await? {
                    report.succeeded += 1;
                    metric.succeed();
                } else {
                    report.lease_lost += 1;
                }
            }
            Err(error) => {
                if fail(
                    pool,
                    &FailJob {
                        id: &job.id,
                        worker,
                        lease_token: &job.lease_token,
                        error_code: error.safe_code,
                        retry_after_seconds: error.retry_after_seconds,
                        max_attempts,
                        permanent: error.permanent,
                    },
                )
                .await?
                {
                    report.retried_or_failed += 1;
                } else {
                    report.lease_lost += 1;
                }
            }
        }
    }
    Ok(report)
}
pub async fn enqueue(pool: &SqlitePool, job: EnqueueJob<'_>) -> Result<String, JobError> {
    if [
        &job.deduplication_key,
        &job.operation_type,
        &job.resource_id,
        &job.destination_grant,
    ]
    .iter()
    .any(|v| v.is_empty())
        || job.resource_version < 0
    {
        return Err(JobError::Invalid);
    }
    let id = Uuid::new_v4().to_string();
    let payload = serde_json::to_string(job.payload).map_err(|_| JobError::Invalid)?;
    sqlx::query("INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) VALUES(?,?,?,?,?,?,?) ON CONFLICT(deduplication_key) DO NOTHING")
  .bind(&id).bind(job.deduplication_key).bind(job.operation_type).bind(job.resource_id).bind(job.resource_version).bind(job.destination_grant).bind(payload).execute(pool).await?;
    let canonical: String =
        sqlx::query_scalar("SELECT id FROM external_jobs WHERE deduplication_key=?")
            .bind(job.deduplication_key)
            .fetch_one(pool)
            .await?;
    Ok(canonical)
}
pub async fn claim(
    pool: &SqlitePool,
    worker: &str,
    lease_seconds: i64,
    limit: i64,
) -> Result<Vec<ClaimedJob>, JobError> {
    if worker.is_empty() || !(1..=3600).contains(&lease_seconds) || !(1..=100).contains(&limit) {
        return Err(JobError::Invalid);
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM external_jobs WHERE (state='pending' AND next_attempt_at<=datetime('now')) OR (state='leased' AND lease_until<datetime('now')) ORDER BY next_attempt_at,id LIMIT ?")
  .bind(limit).fetch_all(&mut *transaction).await?;
    let mut claimed = Vec::new();
    for id in ids {
        let lease_token = Uuid::new_v4().to_string();
        let row=sqlx::query_as::<_,ClaimedJob>("UPDATE external_jobs SET state='leased',lease_owner=?,lease_token=?,lease_until=datetime('now',?),attempt_count=attempt_count+1,updated_at=datetime('now') WHERE id=? AND ((state='pending' AND next_attempt_at<=datetime('now')) OR (state='leased' AND lease_until<datetime('now'))) RETURNING id,lease_owner,lease_token,operation_type,resource_id,resource_version,destination_grant,payload_json,attempt_count")
   .bind(worker).bind(lease_token).bind(format!("+{lease_seconds} seconds")).bind(id).fetch_optional(&mut *transaction).await?;
        if let Some(row) = row {
            claimed.push(row)
        }
    }
    transaction.commit().await?;
    Ok(claimed)
}

pub async fn claim_matching(
    pool: &SqlitePool,
    worker: &str,
    operation_types: &[&str],
    lease_seconds: i64,
    limit: i64,
) -> Result<Vec<ClaimedJob>, JobError> {
    if operation_types.is_empty()
        || operation_types.iter().any(|kind| {
            kind.is_empty()
                || kind.len() > 64
                || !kind
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        || worker.is_empty()
        || !(1..=3600).contains(&lease_seconds)
        || !(1..=100).contains(&limit)
    {
        return Err(JobError::Invalid);
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT candidate.id FROM external_jobs candidate \
         WHERE ((candidate.state='pending' AND candidate.next_attempt_at<=datetime('now')) \
            OR (candidate.state='leased' AND candidate.lease_until<datetime('now'))) \
         AND NOT EXISTS(SELECT 1 FROM external_jobs active \
             WHERE active.resource_id=candidate.resource_id AND active.id<>candidate.id \
               AND active.state='leased' AND active.lease_until>=datetime('now')) \
         AND candidate.operation_type IN (",
    );
    {
        let mut separated = builder.separated(",");
        for operation_type in operation_types {
            separated.push_bind(operation_type);
        }
    }
    builder
        .push(") ORDER BY candidate.next_attempt_at,candidate.id LIMIT ")
        .push_bind(limit);
    let ids: Vec<String> = builder
        .build_query_scalar()
        .fetch_all(&mut *transaction)
        .await?;
    let mut claimed = Vec::new();
    for id in ids {
        let lease_token = Uuid::new_v4().to_string();
        let row=sqlx::query_as::<_,ClaimedJob>("UPDATE external_jobs SET state='leased',lease_owner=?,lease_token=?,lease_until=datetime('now',?),attempt_count=attempt_count+1,updated_at=datetime('now') WHERE id=? AND ((state='pending' AND next_attempt_at<=datetime('now')) OR (state='leased' AND lease_until<datetime('now'))) RETURNING id,lease_owner,lease_token,operation_type,resource_id,resource_version,destination_grant,payload_json,attempt_count")
            .bind(worker).bind(lease_token).bind(format!("+{lease_seconds} seconds")).bind(id)
            .fetch_optional(&mut *transaction).await?;
        if let Some(row) = row {
            claimed.push(row);
        }
    }
    transaction.commit().await?;
    Ok(claimed)
}
pub async fn complete(
    pool: &SqlitePool,
    id: &str,
    worker: &str,
    lease_token: &str,
) -> Result<bool, JobError> {
    Ok(sqlx::query("UPDATE external_jobs SET state='succeeded',lease_owner=NULL,lease_token=NULL,lease_until=NULL,safe_error_code=NULL,updated_at=datetime('now') WHERE id=? AND state='leased' AND lease_owner=? AND lease_token=?")
  .bind(id).bind(worker).bind(lease_token).execute(pool).await?.rows_affected()==1)
}
pub async fn cancel(pool: &SqlitePool, id: &str) -> Result<bool, JobError> {
    Ok(sqlx::query("UPDATE external_jobs SET state='cancelled',lease_owner=NULL,lease_token=NULL,lease_until=NULL,updated_at=datetime('now') WHERE id=? AND state IN ('pending','leased')")
  .bind(id).execute(pool).await?.rows_affected()==1)
}
pub async fn fail(pool: &SqlitePool, failure: &FailJob<'_>) -> Result<bool, JobError> {
    let FailJob {
        id,
        worker,
        lease_token,
        error_code,
        retry_after_seconds,
        max_attempts,
        permanent,
    } = *failure;
    if error_code.is_empty()
        || error_code.len() > 64
        || !error_code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        || max_attempts < 1
    {
        return Err(JobError::Invalid);
    }
    let attempt: Option<i64> = sqlx::query_scalar(
        "SELECT attempt_count FROM external_jobs WHERE id=? AND state='leased' AND lease_owner=? AND lease_token=?",
    )
    .bind(id)
    .bind(worker)
    .bind(lease_token)
    .fetch_optional(pool)
    .await?;
    let Some(attempt) = attempt else {
        return Ok(false);
    };
    let terminal = permanent || attempt >= max_attempts;
    let delay = retry_after_seconds
        .unwrap_or_else(|| retry_delay(id, attempt))
        .min(3600);
    let state = if terminal { "failed" } else { "pending" };
    Ok(sqlx::query("UPDATE external_jobs SET state=?,next_attempt_at=datetime('now',?),lease_owner=NULL,lease_token=NULL,lease_until=NULL,safe_error_code=?,updated_at=datetime('now') WHERE id=? AND state='leased' AND lease_owner=? AND lease_token=?")
  .bind(state).bind(format!("+{delay} seconds")).bind(error_code).bind(id).bind(worker).bind(lease_token).execute(pool).await?.rows_affected()==1)
}

pub async fn renew(
    pool: &SqlitePool,
    id: &str,
    worker: &str,
    lease_token: &str,
    lease_seconds: i64,
) -> Result<bool, JobError> {
    if !(1..=3600).contains(&lease_seconds) {
        return Err(JobError::Invalid);
    }
    Ok(sqlx::query("UPDATE external_jobs SET lease_until=datetime('now',?),updated_at=datetime('now') WHERE id=? AND state='leased' AND lease_owner=? AND lease_token=? AND lease_until>=datetime('now')")
        .bind(format!("+{lease_seconds} seconds")).bind(id).bind(worker).bind(lease_token)
        .execute(pool).await?.rows_affected()==1)
}
fn retry_delay(id: &str, attempt: i64) -> u64 {
    use sha2::{Digest, Sha256};
    let base = 2_u64.saturating_pow(attempt.clamp(1, 11) as u32).min(1800);
    let digest = Sha256::digest(format!("{id}:{attempt}"));
    base + (u64::from(digest[0]) % (base / 4 + 1))
}
#[cfg(test)]
mod tests;
