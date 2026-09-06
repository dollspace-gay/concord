use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct WebhookDispatcher {
    pool: SqlitePool,
    transport: crate::egress::ControlledHttpClient,
    vault: std::sync::Arc<crate::secrets::SecretVault>,
    max_attempts: i64,
}

#[derive(sqlx::FromRow)]
struct DeliveryContext {
    delivery_row_id: String,
    webhook_id: String,
    payload: String,
    grant_version: i64,
    credential_state: String,
    revoked_at: Option<String>,
    url: Option<String>,
    signing_key_id: Option<String>,
    signing_ciphertext: Option<String>,
    server_id: String,
    webhook_channel_id: String,
    event_type: String,
    payload_channel_id: Option<String>,
}

impl WebhookDispatcher {
    pub fn new(
        pool: SqlitePool,
        transport: crate::egress::ControlledHttpClient,
        vault: std::sync::Arc<crate::secrets::SecretVault>,
        max_attempts: i64,
    ) -> Self {
        Self {
            pool,
            transport,
            vault,
            max_attempts,
        }
    }

    async fn dispatch_delivery(
        &self,
        job: &crate::jobs::ClaimedJob,
    ) -> Result<(), crate::jobs::DispatchFailure> {
        if job.operation_type != "webhook_delivery" || self.max_attempts < 1 {
            return Err(permanent("unsupported_operation"));
        }
        let row: Option<DeliveryContext> = sqlx::query_as(
            "SELECT d.id AS delivery_row_id,d.webhook_id,d.payload_json AS payload, \
                    w.grant_version,w.credential_state,w.revoked_at,w.url,w.signing_key_id, \
                    w.signing_ciphertext,w.server_id,w.channel_id AS webhook_channel_id, \
                    d.event_type,json_extract(d.payload_json,'$.channel_id') AS payload_channel_id \
             FROM webhook_deliveries d JOIN webhooks w ON w.id=d.webhook_id \
             WHERE d.delivery_id=? AND d.external_job_id=?",
        )
        .bind(&job.resource_id)
        .bind(&job.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| retryable("webhook_db_unavailable"))?;
        let Some(DeliveryContext {
            delivery_row_id,
            webhook_id,
            payload,
            grant_version,
            credential_state,
            revoked_at,
            url,
            signing_key_id,
            signing_ciphertext,
            server_id,
            webhook_channel_id,
            event_type,
            payload_channel_id,
        }) = row
        else {
            return Err(permanent("webhook_delivery_missing"));
        };
        let source_is_eligible: bool = if payload_channel_id.as_deref()
            != Some(webhook_channel_id.as_str())
        {
            false
        } else {
            sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM channels c WHERE c.id=? AND c.server_id=? \
                 AND c.is_private=0) AND (?='webhook_test' OR EXISTS(SELECT 1 FROM webhook_events \
                 WHERE webhook_id=? AND event_type=?)) AND \
                 (? IN ('webhook_test','message_delete') OR EXISTS(SELECT 1 FROM messages m \
                   WHERE m.id=json_extract(?,'$.entity_id') AND m.channel_id=? AND m.deleted_at IS NULL))",
                )
                .bind(&webhook_channel_id)
                .bind(&server_id)
                .bind(&event_type)
                .bind(&webhook_id)
                .bind(&event_type)
                .bind(&event_type)
                .bind(&payload)
                .bind(&webhook_channel_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| retryable("webhook_db_unavailable"))?
        };
        if job.destination_grant != format!("webhook:{webhook_id}:{grant_version}")
            || credential_state != "active"
            || revoked_at.is_some()
            || !source_is_eligible
        {
            self.record_failure(&delivery_row_id, job, "webhook_scope_revoked", true, None)
                .await?;
            return Err(permanent("webhook_scope_revoked"));
        }
        let Some(url) = url.and_then(|url| reqwest::Url::parse(&url).ok()) else {
            self.record_failure(
                &delivery_row_id,
                job,
                "webhook_destination_invalid",
                true,
                None,
            )
            .await?;
            return Err(permanent("webhook_destination_invalid"));
        };
        let (Some(key_id), Some(ciphertext)) = (signing_key_id, signing_ciphertext) else {
            self.record_failure(&delivery_row_id, job, "webhook_secret_missing", true, None)
                .await?;
            return Err(permanent("webhook_secret_missing"));
        };
        let context = format!("webhook:{webhook_id}:signing");
        let secret = match self.vault.decrypt(&context, &ciphertext, &key_id) {
            Ok(secret) => secret,
            Err(_) => {
                self.record_failure(
                    &delivery_row_id,
                    job,
                    "webhook_secret_unavailable",
                    true,
                    None,
                )
                .await?;
                return Err(permanent("webhook_secret_unavailable"));
            }
        };
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let mut signed = timestamp.as_bytes().to_vec();
        signed.push(b'.');
        signed.extend_from_slice(payload.as_bytes());
        let signature = hmac_sha256(&secret, &signed);
        let request = self
            .transport
            .request(
                reqwest::Method::POST,
                url.clone(),
                crate::egress::RedirectPolicy::Reject,
            )
            .and_then(|request| {
                Ok(request
                    .credentials_for(&url)?
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        reqwest::header::HeaderValue::from_static("application/json"),
                    )
                    .header(
                        reqwest::header::HeaderName::from_static("x-concord-timestamp"),
                        reqwest::header::HeaderValue::from_str(&timestamp).map_err(|_| {
                            crate::egress::EgressError::InvalidRequest("invalid timestamp")
                        })?,
                    )
                    .header(
                        reqwest::header::HeaderName::from_static("x-concord-signature-256"),
                        reqwest::header::HeaderValue::from_str(&format!(
                            "sha256={}",
                            hex::encode(signature)
                        ))
                        .map_err(|_| {
                            crate::egress::EgressError::InvalidRequest("invalid signature")
                        })?,
                    )
                    .header(
                        reqwest::header::HeaderName::from_static("x-concord-delivery"),
                        reqwest::header::HeaderValue::from_str(&job.resource_id).map_err(|_| {
                            crate::egress::EgressError::InvalidRequest("invalid delivery id")
                        })?,
                    )
                    .body(payload.as_bytes().to_vec()))
            });
        let response = match request {
            Ok(request) => {
                let pool = self.pool.clone();
                let delivery = delivery_row_id.clone();
                let job_id = job.id.clone();
                let destination_grant = job.destination_grant.clone();
                self.transport
                    .send_with_preflight(request, move || async move {
                        let eligible: bool = sqlx::query_scalar(
                            "SELECT EXISTS(SELECT 1 FROM webhook_deliveries d \
                             JOIN webhooks w ON w.id=d.webhook_id \
                             JOIN channels c ON c.id=w.channel_id AND c.server_id=w.server_id \
                             WHERE d.id=? AND d.external_job_id=? AND w.credential_state='active' \
                               AND w.revoked_at IS NULL AND c.is_private=0 \
                               AND ?=('webhook:'||w.id||':'||w.grant_version) \
                               AND json_extract(d.payload_json,'$.channel_id')=w.channel_id \
                               AND (d.event_type='webhook_test' OR EXISTS(SELECT 1 FROM webhook_events e \
                                 WHERE e.webhook_id=w.id AND e.event_type=d.event_type)) \
                               AND (d.event_type IN ('webhook_test','message_delete') OR EXISTS( \
                                 SELECT 1 FROM messages m WHERE m.id=json_extract(d.payload_json,'$.entity_id') \
                                   AND m.channel_id=w.channel_id AND m.deleted_at IS NULL)))",
                        )
                        .bind(delivery)
                        .bind(job_id)
                        .bind(destination_grant)
                        .fetch_one(&pool)
                        .await
                        .map_err(|_| crate::egress::EgressError::Protocol)?;
                        if eligible {
                            Ok(())
                        } else {
                            Err(crate::egress::EgressError::InvalidRequest(
                                "webhook preflight rejected",
                            ))
                        }
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        match response {
            Ok(response) if response.status.is_success() => {
                let changed = sqlx::query(
                    "UPDATE webhook_deliveries SET state='delivered',attempt_count=?, \
                     last_status=?,safe_error_code=NULL,delivered_at=datetime('now') WHERE id=? \
                     AND EXISTS(SELECT 1 FROM external_jobs WHERE id=? AND state='leased' \
                       AND lease_owner=? AND lease_token=? AND lease_until>=datetime('now'))",
                )
                .bind(job.attempt_count)
                .bind(i64::from(response.status.as_u16()))
                .bind(&delivery_row_id)
                .bind(&job.id)
                .bind(&job.lease_owner)
                .bind(&job.lease_token)
                .execute(&self.pool)
                .await
                .map_err(|_| retryable("webhook_db_unavailable"))?;
                if changed.rows_affected() != 1 {
                    return Err(retryable("webhook_db_unavailable"));
                }
                sqlx::query(
                    "UPDATE webhooks SET last_delivery_at=datetime('now'), \
                     last_safe_error_code=NULL WHERE id=?",
                )
                .bind(&webhook_id)
                .execute(&self.pool)
                .await
                .map_err(|_| retryable("webhook_db_unavailable"))?;
                Ok(())
            }
            Ok(response) => {
                let status = response.status.as_u16();
                let is_retryable = status == 408 || status == 429 || status >= 500;
                let code = if is_retryable {
                    "webhook_http_retryable"
                } else {
                    "webhook_http_rejected"
                };
                self.record_failure(&delivery_row_id, job, code, !is_retryable, Some(status))
                    .await?;
                Err(crate::jobs::DispatchFailure {
                    safe_code: code,
                    retry_after_seconds: retry_after(&response.headers),
                    permanent: !is_retryable,
                })
            }
            Err(crate::egress::EgressError::InvalidRequest("webhook preflight rejected")) => {
                self.record_failure(&delivery_row_id, job, "webhook_scope_revoked", true, None)
                    .await?;
                Err(permanent("webhook_scope_revoked"))
            }
            Err(_) => {
                self.record_failure(
                    &delivery_row_id,
                    job,
                    "webhook_transport_unavailable",
                    false,
                    None,
                )
                .await?;
                Err(retryable("webhook_transport_unavailable"))
            }
        }
    }

    async fn record_failure(
        &self,
        delivery_id: &str,
        job: &crate::jobs::ClaimedJob,
        code: &'static str,
        permanent_failure: bool,
        status: Option<u16>,
    ) -> Result<(), crate::jobs::DispatchFailure> {
        let terminal = permanent_failure || job.attempt_count >= self.max_attempts;
        let changed = sqlx::query(
            "UPDATE webhook_deliveries SET state=?,attempt_count=?,last_status=?, \
             safe_error_code=? WHERE id=? AND EXISTS(SELECT 1 FROM external_jobs \
             WHERE id=? AND state='leased' AND lease_owner=? AND lease_token=? \
               AND lease_until>=datetime('now'))",
        )
        .bind(if terminal { "failed" } else { "pending" })
        .bind(job.attempt_count)
        .bind(status.map(i64::from))
        .bind(code)
        .bind(delivery_id)
        .bind(&job.id)
        .bind(&job.lease_owner)
        .bind(&job.lease_token)
        .execute(&self.pool)
        .await
        .map_err(|_| retryable("webhook_db_unavailable"))?;
        if changed.rows_affected() != 1 {
            return Err(retryable("webhook_lease_lost"));
        }
        sqlx::query("UPDATE webhooks SET last_safe_error_code=? WHERE id=(SELECT webhook_id FROM webhook_deliveries WHERE id=?)")
            .bind(code).bind(delivery_id).execute(&self.pool).await
            .map_err(|_| retryable("webhook_db_unavailable"))?;
        Ok(())
    }
}

impl crate::jobs::JobDispatcher for WebhookDispatcher {
    fn dispatch<'a>(
        &'a self,
        job: &'a crate::jobs::ClaimedJob,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), crate::jobs::DispatchFailure>> + Send + 'a>,
    > {
        Box::pin(self.dispatch_delivery(job))
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.min(3600))
}

fn retryable(code: &'static str) -> crate::jobs::DispatchFailure {
    crate::jobs::DispatchFailure {
        safe_code: code,
        retry_after_seconds: None,
        permanent: false,
    }
}

fn permanent(code: &'static str) -> crate::jobs::DispatchFailure {
    crate::jobs::DispatchFailure {
        safe_code: code,
        retry_after_seconds: None,
        permanent: true,
    }
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut normalized = [0_u8; BLOCK];
    if key.len() > BLOCK {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK];
    let mut outer_pad = [0x5c_u8; BLOCK];
    for index in 0..BLOCK {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    outer.finalize().into()
}

#[cfg(test)]
mod tests;
