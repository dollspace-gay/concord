use super::{Result, Uuid, bail, insert_operator_audit, validate_operator_reason};
use anyhow::Context;

pub(super) async fn print_external_jobs(
    pool: &sqlx::SqlitePool,
    state: Option<&str>,
    limit: i64,
) -> Result<()> {
    const STATES: &[&str] = &["pending", "leased", "succeeded", "failed", "cancelled"];
    if !(1..=500).contains(&limit) {
        bail!("job inspection limit must be between 1 and 500");
    }
    if state.is_some_and(|value| !STATES.contains(&value)) {
        bail!("job state filter is invalid");
    }
    type JobInventoryRow = (
        String,
        String,
        String,
        i64,
        String,
        i64,
        Option<String>,
        String,
        String,
    );
    let rows: Vec<JobInventoryRow> = if let Some(state) = state {
        sqlx::query_as(
            "SELECT id,operation_type,resource_id,resource_version,state,attempt_count, \
                    safe_error_code,next_attempt_at,updated_at \
             FROM external_jobs WHERE state=? ORDER BY updated_at,id LIMIT ?",
        )
        .bind(state)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT id,operation_type,resource_id,resource_version,state,attempt_count, \
                    safe_error_code,next_attempt_at,updated_at \
             FROM external_jobs ORDER BY updated_at,id LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    for row in rows {
        println!(
            "{}",
            serde_json::json!({
                "id": row.0,
                "operation_type": row.1,
                "resource_id": row.2,
                "resource_version": row.3,
                "state": row.4,
                "attempt_count": row.5,
                "safe_error_code": row.6,
                "next_attempt_at": row.7,
                "updated_at": row.8,
            })
        );
    }
    Ok(())
}

pub(super) async fn retry_external_job(
    pool: &sqlx::SqlitePool,
    job_id: &str,
    reason: &str,
) -> Result<()> {
    let reason = validate_operator_reason(reason)?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: (String, String) = sqlx::query_as(
        "SELECT operation_type,resource_id FROM external_jobs WHERE id=? AND state='failed'",
    )
    .bind(job_id)
    .fetch_optional(&mut *transaction)
    .await?
    .with_context(|| format!("failed external job was not found: {job_id}"))?;
    if row.0 != "webhook_delivery" {
        if matches!(
            row.0.as_str(),
            "atproto_publish" | "atproto_update" | "atproto_delete"
        ) {
            bail!(
                "AT publication jobs require atproto-publication-reconcile so uncertain remote state and current grants are checked"
            );
        }
        bail!(
            "external job type is not eligible for operator retry: {}",
            row.0
        );
    }
    let eligible: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM webhook_deliveries d \
           JOIN webhooks w ON w.id=d.webhook_id \
           JOIN channels c ON c.id=w.channel_id AND c.server_id=w.server_id \
           WHERE d.external_job_id=? AND d.delivery_id=? AND d.state='failed' \
             AND w.webhook_type='outgoing' AND w.credential_state='active' \
             AND w.revoked_at IS NULL AND w.url IS NOT NULL AND c.is_private=0)",
    )
    .bind(job_id)
    .bind(&row.1)
    .fetch_one(&mut *transaction)
    .await?;
    if eligible != 1 {
        bail!("external job is no longer eligible under its current webhook grant");
    }
    let changed = sqlx::query(
        "UPDATE external_jobs SET state='pending',next_attempt_at=datetime('now'), \
                lease_owner=NULL,lease_token=NULL,lease_until=NULL,safe_error_code=NULL, \
                updated_at=datetime('now') \
         WHERE id=? AND state='failed'",
    )
    .bind(job_id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        bail!("failed external job changed while retry was admitted");
    }
    let delivery = sqlx::query(
        "UPDATE webhook_deliveries SET state='pending',last_status=NULL,safe_error_code=NULL \
         WHERE external_job_id=? AND delivery_id=? AND state='failed'",
    )
    .bind(job_id)
    .bind(&row.1)
    .execute(&mut *transaction)
    .await?;
    if delivery.rows_affected() != 1 {
        bail!("matching failed webhook delivery was not found");
    }
    insert_operator_audit(
        &mut transaction,
        "external_job_retry",
        "external_job",
        job_id,
        reason,
        &serde_json::json!({"operation_type": row.0, "resource_id": row.1}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) async fn reconcile_atproto_publication(
    pool: &sqlx::SqlitePool,
    publication_id: &str,
) -> Result<String> {
    use sqlx::Row;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query(
        "SELECT p.user_id,p.source_version,p.remote_uri,m.deleted_at,m.sender_id,m.channel_id,
                g.grant_version,c.atproto_publication_enabled,c.is_private,
                c.visibility_repair_required,c.parent_channel_id,c.channel_type,
                EXISTS(SELECT 1 FROM oauth_accounts oa WHERE oa.user_id=p.user_id
                  AND oa.provider='atproto' AND oa.credential_state='active')
         FROM atproto_publications p JOIN messages m ON m.id=p.source_message_id
         JOIN channels c ON c.id=m.channel_id
         LEFT JOIN atproto_publication_grants g ON g.user_id=p.user_id
              AND g.channel_id=m.channel_id AND g.enabled=1
         WHERE p.id=? AND p.status='failed'",
    )
    .bind(publication_id)
    .fetch_optional(&mut *transaction)
    .await?
    .with_context(|| format!("failed publication {publication_id} was not found"))?;
    let user_id: String = row.get(0);
    if row.get::<String, _>(4) != user_id || !row.get::<bool, _>(12) {
        bail!("publication source ownership or AT credential is no longer valid");
    }
    let deleted = row.get::<Option<String>, _>(3).is_some();
    let operation = if deleted {
        "atproto_delete"
    } else {
        let eligible = row.get::<Option<i64>, _>(6).is_some()
            && row.get::<i64, _>(7) == 1
            && row.get::<i64, _>(8) == 0
            && row.get::<i64, _>(9) == 0
            && row.get::<Option<String>, _>(10).is_none()
            && !matches!(
                row.get::<String, _>(11).as_str(),
                "public_thread" | "private_thread"
            );
        if !eligible {
            bail!("publication is no longer eligible under current channel/user grant");
        }
        if row.get::<Option<String>, _>(2).is_some() {
            "atproto_update"
        } else {
            "atproto_publish"
        }
    };
    let source_version: i64 = row.get(1);
    let status = match operation {
        "atproto_delete" => "delete_pending",
        "atproto_update" => "update_pending",
        _ => "pending",
    };
    let job_id = Uuid::new_v4().to_string();
    sqlx::query("UPDATE atproto_publications SET status=?,safe_error_code=NULL,updated_at=datetime('now') WHERE id=? AND status='failed'")
        .bind(status).bind(publication_id).execute(&mut *transaction).await?;
    sqlx::query(
        "INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,
          resource_version,destination_grant,payload_json) VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&job_id)
    .bind(format!(
        "atproto-publication:{publication_id}:{source_version}:operator:{job_id}"
    ))
    .bind(operation)
    .bind(publication_id)
    .bind(source_version)
    .bind(format!(
        "atproto-user:{user_id}:{}",
        row.get::<Option<i64>, _>(6).unwrap_or(0)
    ))
    .bind(
        serde_json::json!({"publication_id":publication_id,"reconcile":true,"operator":true})
            .to_string(),
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(status.to_owned())
}
