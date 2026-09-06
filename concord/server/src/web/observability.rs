use std::fmt::Write as _;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::FromRow;
use tokio::io::AsyncWriteExt as _;

use super::app_state::AppState;
use super::auth_middleware::AuthUser;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const METRICS_CACHE_AGE: Duration = Duration::from_secs(1);
const METRICS_STALE_AGE: Duration = Duration::from_secs(30);
const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

pub async fn health_live() -> StatusCode {
    StatusCode::OK
}

pub async fn health_ready(State(state): State<Arc<AppState>>) -> StatusCode {
    if !state.health.is_ready() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    let health = Arc::clone(&state.health);
    let Some(receiver) =
        spawn_dependency_probe(&health, async move { readiness_dependencies(&state).await })
    else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    match tokio::time::timeout(PROBE_TIMEOUT, receiver).await {
        Ok(Ok(Ok(()))) => StatusCode::OK,
        Ok(Ok(Err(error))) => {
            tracing::warn!(%error, "readiness dependency probe failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
        Ok(Err(_)) => StatusCode::SERVICE_UNAVAILABLE,
        Err(_) => {
            tracing::warn!("readiness dependency probe timed out");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

fn spawn_dependency_probe<F>(
    health: &Arc<super::app_state::HealthState>,
    work: F,
) -> Option<tokio::sync::oneshot::Receiver<Result<(), String>>>
where
    F: Future<Output = Result<(), String>> + Send + 'static,
{
    let ownership = health.begin_dependency_probe()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ownership = ownership;
        let mut timer =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::ReadinessProbe);
        let result = work.await;
        if result.is_ok() {
            timer.succeed();
        }
        if let Err(Err(error)) = sender.send(result) {
            tracing::warn!(%error, "readiness dependency probe failed after requester disconnected");
        }
    });
    Some(receiver)
}

async fn readiness_dependencies(state: &AppState) -> Result<(), String> {
    let schema_version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version),0) FROM schema_version")
            .fetch_one(&state.db)
            .await
            .map_err(|error| format!("database read failed: {error}"))?;
    let expected = crate::db::pool::current_schema_version();
    if schema_version != expected {
        return Err(format!(
            "database schema is {schema_version}, expected {expected}"
        ));
    }

    let (_permit, mut transaction) = state.engine.begin_admitted_write().await?;
    sqlx::query("UPDATE event_retention_state SET updated_at=datetime('now') WHERE singleton=1")
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("database write probe failed: {error}"))?;
    transaction
        .rollback()
        .await
        .map_err(|error| format!("database probe rollback failed: {error}"))?;

    probe_media_storage(&state.media_dir).await
}

async fn probe_media_storage(root: &std::path::Path) -> Result<(), String> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| format!("media directory unavailable: {error}"))?;
    let path = root.join(format!(".concord-readiness-{}", uuid::Uuid::new_v4()));
    let probe_result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|error| format!("media write unavailable: {error}"))?;
        file.write_all(b"concord-readiness\n")
            .await
            .map_err(|error| format!("media write failed: {error}"))?;
        file.sync_all()
            .await
            .map_err(|error| format!("media sync failed: {error}"))?;
        drop(file);
        tokio::fs::remove_file(&path)
            .await
            .map_err(|error| format!("media cleanup failed: {error}"))?;
        let directory = tokio::fs::File::open(root)
            .await
            .map_err(|error| format!("media directory open failed: {error}"))?;
        directory
            .sync_all()
            .await
            .map_err(|error| format!("media directory sync failed: {error}"))
    }
    .await;
    if probe_result.is_err() {
        let _ = tokio::fs::remove_file(path).await;
    }
    probe_result
}

#[derive(FromRow)]
struct MetricsRow {
    schema_version: i64,
    command_receipts: i64,
    retained_events: i64,
    event_high_water: i64,
    dispatcher_high_water: i64,
    delivery_pending: i64,
    delivery_oldest_age_seconds: i64,
    attachments_staging: i64,
    attachments_ready: i64,
    attachments_attached: i64,
    attachments_deleting: i64,
    attachments_deleted: i64,
    attachments_failed: i64,
    attachments_legacy_external: i64,
    jobs_pending: i64,
    jobs_leased: i64,
    jobs_succeeded: i64,
    jobs_failed: i64,
    jobs_cancelled: i64,
    job_attempts: i64,
    active_web_sessions: i64,
}

async fn collect_metrics(pool: &sqlx::SqlitePool) -> Result<MetricsRow, sqlx::Error> {
    sqlx::query_as(
        "SELECT \
         (SELECT COALESCE(MAX(version),0) FROM schema_version) AS schema_version, \
         (SELECT COUNT(*) FROM command_receipts) AS command_receipts, \
         (SELECT COUNT(*) FROM event_log) AS retained_events, \
         (SELECT COALESCE(MAX(event_sequence),0) FROM event_log) AS event_high_water, \
         (SELECT dispatcher_high_water FROM event_retention_state WHERE singleton=1) AS dispatcher_high_water, \
         (SELECT COUNT(*) FROM delivery_outbox WHERE completed_at IS NULL) AS delivery_pending, \
         (SELECT MAX(0,COALESCE(MAX(unixepoch()-unixepoch(available_at)),0)) FROM delivery_outbox WHERE completed_at IS NULL) AS delivery_oldest_age_seconds, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='staging') AS attachments_staging, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='ready') AS attachments_ready, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='attached') AS attachments_attached, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='deleting') AS attachments_deleting, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='deleted') AS attachments_deleted, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='failed') AS attachments_failed, \
         (SELECT COUNT(*) FROM attachments WHERE media_state='legacy_external') AS attachments_legacy_external, \
         (SELECT COUNT(*) FROM external_jobs WHERE state='pending') AS jobs_pending, \
         (SELECT COUNT(*) FROM external_jobs WHERE state='leased') AS jobs_leased, \
         (SELECT COUNT(*) FROM external_jobs WHERE state='succeeded') AS jobs_succeeded, \
         (SELECT COUNT(*) FROM external_jobs WHERE state='failed') AS jobs_failed, \
         (SELECT COUNT(*) FROM external_jobs WHERE state='cancelled') AS jobs_cancelled, \
         (SELECT COALESCE(SUM(attempt_count),0) FROM external_jobs) AS job_attempts, \
         (SELECT COUNT(*) FROM auth_credentials WHERE kind='web_session' AND revoked_at IS NULL AND expires_at>unixepoch()) AS active_web_sessions",
    )
    .fetch_one(pool)
    .await
}

pub async fn metrics(State(state): State<Arc<AppState>>, auth: AuthUser) -> Response {
    match crate::db::queries::servers::is_system_admin(&state.db, &auth.user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return (StatusCode::FORBIDDEN, "System administrator required").into_response();
        }
        Err(error) => {
            tracing::warn!(%error, "metrics authorization lookup failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Authorization service unavailable",
            )
                .into_response();
        }
    }
    if let Some((status, body)) = state.health.cached_metrics(METRICS_CACHE_AGE) {
        return metrics_response(
            StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
            body,
        );
    }
    let Some(receiver) = spawn_metrics_collection(state.clone()) else {
        if let Some((status, body)) = state.health.cached_metrics(METRICS_STALE_AGE) {
            return metrics_response(
                StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
                body,
            );
        }
        return metrics_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "concord_metrics_collection_success 0\n".into(),
        );
    };
    match tokio::time::timeout(PROBE_TIMEOUT, receiver).await {
        Ok(Ok((status, body))) => metrics_response(status, body),
        Ok(Err(_)) | Err(_) => metrics_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "concord_metrics_collection_success 0\n".into(),
        ),
    }
}

fn spawn_metrics_collection(
    state: Arc<AppState>,
) -> Option<tokio::sync::oneshot::Receiver<(StatusCode, String)>> {
    let health = state.health.clone();
    spawn_metrics_work(&health, async move {
        let mut timer = crate::runtime_metrics::Timer::start(
            crate::runtime_metrics::Operation::MetricsCollection,
        );
        let started = Instant::now();
        let snapshot = collect_metrics(&state.db).await;
        if snapshot.is_ok() {
            timer.succeed();
        }
        let elapsed = started.elapsed().as_secs_f64();
        match snapshot {
            Ok(row) => (StatusCode::OK, render_metrics(&state, Some(&row), elapsed)),
            Err(error) => {
                tracing::warn!(%error, "metrics database collection failed");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    render_metrics(&state, None, elapsed),
                )
            }
        }
    })
}

fn spawn_metrics_work<F>(
    health: &Arc<super::app_state::HealthState>,
    work: F,
) -> Option<tokio::sync::oneshot::Receiver<(StatusCode, String)>>
where
    F: Future<Output = (StatusCode, String)> + Send + 'static,
{
    let ownership = health.begin_metrics_collection()?;
    let health = health.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ownership = ownership;
        let (status, body) = work.await;
        health.store_metrics(status.as_u16(), body.clone());
        let _ = sender.send((status, body));
    });
    Some(receiver)
}

fn render_metrics(state: &AppState, snapshot: Option<&MetricsRow>, elapsed: f64) -> String {
    let health = state.health.snapshot();
    let mut output = String::with_capacity(4_096);
    output.push_str("# HELP concord_metrics_collection_success Whether this scrape collected the database snapshot.\n");
    output.push_str("# TYPE concord_metrics_collection_success gauge\n");
    output.push_str("# HELP concord_metrics_collection_duration_seconds Time spent collecting this metrics snapshot.\n");
    output.push_str("# TYPE concord_metrics_collection_duration_seconds gauge\n");
    let _ = writeln!(
        output,
        "concord_metrics_collection_duration_seconds {elapsed:.6}"
    );
    output.push_str("# HELP concord_health_component_ready Whether a required local service component is ready.\n");
    output.push_str("# TYPE concord_health_component_ready gauge\n");
    let _ = writeln!(
        output,
        "concord_health_component_ready{{component=\"request_admission\"}} {}",
        i32::from(health.accepting_requests)
    );
    let _ = writeln!(
        output,
        "concord_health_component_ready{{component=\"web_listener\"}} {}",
        i32::from(health.web_listener_bound)
    );
    let _ = writeln!(
        output,
        "concord_health_component_ready{{component=\"irc_listener\"}} {}",
        i32::from(health.irc_listener_bound)
    );
    output.push_str(
        "# HELP concord_upload_admission_available Currently available upload permits.\n",
    );
    output.push_str("# TYPE concord_upload_admission_available gauge\n");
    let _ = writeln!(
        output,
        "concord_upload_admission_available {}",
        state.upload_admission.available_permits()
    );
    append_runtime_metrics(&mut output);
    if let Some(row) = snapshot {
        output.push_str("concord_metrics_collection_success 1\n");
        append_database_metrics(&mut output, row);
    } else {
        output.push_str("concord_metrics_collection_success 0\n");
    }
    output
}

fn metrics_response(status: StatusCode, body: String) -> Response {
    (status, [(header::CONTENT_TYPE, METRICS_CONTENT_TYPE)], body).into_response()
}

fn append_runtime_metrics(output: &mut String) {
    let snapshot = crate::runtime_metrics::snapshot();
    output.push_str("# HELP concord_runtime_operations_total Completed runtime operations by fixed operation and outcome.\n");
    output.push_str("# TYPE concord_runtime_operations_total counter\n");
    output
        .push_str("# HELP concord_runtime_operation_duration_seconds Runtime operation latency.\n");
    output.push_str("# TYPE concord_runtime_operation_duration_seconds histogram\n");
    for operation in crate::runtime_metrics::Operation::ALL {
        let index = operation as usize;
        let name = operation.name();
        let _ = writeln!(
            output,
            "concord_runtime_operations_total{{operation=\"{name}\",outcome=\"success\"}} {}",
            snapshot.succeeded[index]
        );
        let _ = writeln!(
            output,
            "concord_runtime_operations_total{{operation=\"{name}\",outcome=\"failure\"}} {}",
            snapshot.failed[index]
        );
        for (bucket, count) in snapshot.duration_buckets[index].iter().enumerate() {
            let boundary = crate::runtime_metrics::bucket_name(bucket);
            let _ = writeln!(
                output,
                "concord_runtime_operation_duration_seconds_bucket{{operation=\"{name}\",le=\"{boundary}\"}} {count}"
            );
        }
        let _ = writeln!(
            output,
            "concord_runtime_operation_duration_seconds_sum{{operation=\"{name}\"}} {:.9}",
            snapshot.duration_seconds[index]
        );
        let _ = writeln!(
            output,
            "concord_runtime_operation_duration_seconds_count{{operation=\"{name}\"}} {}",
            snapshot.duration_count[index]
        );
    }
}

fn append_database_metrics(output: &mut String, row: &MetricsRow) {
    output.push_str("# HELP concord_database_schema_version Applied database schema version.\n");
    output.push_str("# TYPE concord_database_schema_version gauge\n");
    let _ = writeln!(
        output,
        "concord_database_schema_version {}",
        row.schema_version
    );
    output.push_str("# HELP concord_command_receipts_total Durable command receipts retained.\n");
    output.push_str("# TYPE concord_command_receipts_total gauge\n");
    let _ = writeln!(
        output,
        "concord_command_receipts_total {}",
        row.command_receipts
    );
    output.push_str("# HELP concord_event_log_retained Durable events currently retained.\n");
    output.push_str("# TYPE concord_event_log_retained gauge\n");
    let _ = writeln!(output, "concord_event_log_retained {}", row.retained_events);
    output.push_str("# HELP concord_event_sequence_high_water Highest retained event sequence.\n");
    output.push_str("# TYPE concord_event_sequence_high_water gauge\n");
    let _ = writeln!(
        output,
        "concord_event_sequence_high_water {}",
        row.event_high_water
    );
    output.push_str("# HELP concord_delivery_dispatcher_high_water Highest event sequence completed by durable delivery.\n");
    output.push_str("# TYPE concord_delivery_dispatcher_high_water gauge\n");
    let _ = writeln!(
        output,
        "concord_delivery_dispatcher_high_water {}",
        row.dispatcher_high_water
    );
    output.push_str(
        "# HELP concord_delivery_replay_lag_events Events beyond the dispatcher high water.\n",
    );
    output.push_str("# TYPE concord_delivery_replay_lag_events gauge\n");
    let _ = writeln!(
        output,
        "concord_delivery_replay_lag_events {}",
        row.event_high_water
            .saturating_sub(row.dispatcher_high_water)
    );
    output.push_str("# HELP concord_delivery_outbox_pending Pending durable delivery rows.\n");
    output.push_str("# TYPE concord_delivery_outbox_pending gauge\n");
    let _ = writeln!(
        output,
        "concord_delivery_outbox_pending {}",
        row.delivery_pending
    );
    output.push_str(
        "# HELP concord_delivery_outbox_oldest_age_seconds Age of the oldest pending delivery.\n",
    );
    output.push_str("# TYPE concord_delivery_outbox_oldest_age_seconds gauge\n");
    let _ = writeln!(
        output,
        "concord_delivery_outbox_oldest_age_seconds {}",
        row.delivery_oldest_age_seconds
    );
    output.push_str("# HELP concord_attachments Attachments in each fixed lifecycle state.\n");
    output.push_str("# TYPE concord_attachments gauge\n");
    for (state, count) in [
        ("staging", row.attachments_staging),
        ("ready", row.attachments_ready),
        ("attached", row.attachments_attached),
        ("deleting", row.attachments_deleting),
        ("deleted", row.attachments_deleted),
        ("failed", row.attachments_failed),
        ("legacy_external", row.attachments_legacy_external),
    ] {
        let _ = writeln!(output, "concord_attachments{{state=\"{state}\"}} {count}");
    }
    output.push_str("# HELP concord_external_jobs External jobs in each fixed lifecycle state.\n");
    output.push_str("# TYPE concord_external_jobs gauge\n");
    for (state, count) in [
        ("pending", row.jobs_pending),
        ("leased", row.jobs_leased),
        ("succeeded", row.jobs_succeeded),
        ("failed", row.jobs_failed),
        ("cancelled", row.jobs_cancelled),
    ] {
        let _ = writeln!(output, "concord_external_jobs{{state=\"{state}\"}} {count}");
    }
    output.push_str(
        "# HELP concord_external_job_attempts_total Total persisted external job attempts.\n",
    );
    output.push_str("# TYPE concord_external_job_attempts_total gauge\n");
    let _ = writeln!(
        output,
        "concord_external_job_attempts_total {}",
        row.job_attempts
    );
    output.push_str(
        "# HELP concord_active_web_sessions Unrevoked and unexpired web session credentials.\n",
    );
    output.push_str("# TYPE concord_active_web_sessions gauge\n");
    let _ = writeln!(
        output,
        "concord_active_web_sessions {}",
        row.active_web_sessions
    );
}

#[cfg(test)]
mod tests;
