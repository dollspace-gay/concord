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
mod tests {
    use super::{WebhookDispatcher, hmac_sha256};
    use crate::db::pool::{create_pool, run_migrations};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn queued_delivery_fixture(
        address: std::net::SocketAddr,
    ) -> (
        sqlx::SqlitePool,
        std::sync::Arc<crate::secrets::SecretVault>,
        crate::egress::ControlledHttpClient,
    ) {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','owner','owner','secret-at-queue-time')")
            .execute(&pool).await.unwrap();
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([43_u8; 32])).unwrap();
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
        let ciphertext = vault.encrypt("webhook:hook:signing", b"secret").unwrap();
        sqlx::query("INSERT INTO webhooks(id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state,signing_key_id,signing_ciphertext) VALUES('hook','server','channel','Hook','outgoing','hash',?,'owner','active',?,?)")
            .bind(format!("http://{address}/delivery")).bind(vault.key_id()).bind(ciphertext).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO webhook_events(id,webhook_id,event_type) VALUES('event','hook','message_create')")
            .execute(&pool).await.unwrap();
        let payload = serde_json::json!({"delivery_id":"delivery","event_type":"message_create","entity_id":"message","channel_id":"channel","data":{"content":"secret-at-queue-time"}}).to_string();
        sqlx::query("INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) VALUES('job','dedupe','webhook_delivery','delivery',1,'webhook:hook:1',?)")
            .bind(&payload).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO webhook_deliveries(id,webhook_id,external_job_id,delivery_id,event_type,event_version,payload_json) VALUES('row','hook','job','delivery','message_create',1,?)")
            .bind(payload).execute(&pool).await.unwrap();
        let origin = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        let transport =
            crate::egress::ControlledHttpClient::scoped_origins(&[origin], 4096, 1).unwrap();
        (pool, vault, transport)
    }

    async fn run_http_failure(status: u16, prior_attempts: i64) -> (String, String, i64, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = socket.read(&mut buffer).await.unwrap();
            socket
                .write_all(
                    format!("HTTP/1.1 {status} Failure\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
        });
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([31_u8; 32])).unwrap();
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
        let ciphertext = vault
            .encrypt("webhook:hook:signing", b"delivery-secret")
            .unwrap();
        sqlx::query("INSERT INTO webhooks(id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state,signing_key_id,signing_ciphertext) VALUES('hook','server','channel','Hook','outgoing','hash',?,'owner','active',?,?)")
            .bind(format!("http://{address}/delivery")).bind(vault.key_id()).bind(ciphertext)
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO webhook_events(webhook_id,event_type) VALUES('hook','message_create')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','owner','owner','body')")
            .execute(&pool).await.unwrap();
        let payload = serde_json::json!({"event_type":"message_create","channel_id":"channel","entity_id":"message"}).to_string();
        sqlx::query("INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json,attempt_count) VALUES('job','dedupe','webhook_delivery','delivery',1,'webhook:hook:1',?,?)")
            .bind(&payload).bind(prior_attempts).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO webhook_deliveries(id,webhook_id,external_job_id,delivery_id,event_type,event_version,payload_json,attempt_count) VALUES('row','hook','job','delivery','message_create',1,?,?)")
            .bind(&payload).bind(prior_attempts).execute(&pool).await.unwrap();
        let origin = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        let transport =
            crate::egress::ControlledHttpClient::scoped_origins(&[origin], 4096, 1).unwrap();
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
        receiver.await.unwrap();
        sqlx::query_as("SELECT d.state,j.state,d.attempt_count,d.safe_error_code FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id")
            .fetch_one(&pool).await.unwrap()
    }

    #[test]
    fn hmac_matches_rfc_4231_vector() {
        assert_eq!(
            hex::encode(hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[tokio::test]
    async fn worker_sends_signed_payload_and_commits_delivery_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0_u8; 2048];
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..header_end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([29_u8; 32])).unwrap();
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
        let secret = b"delivery-secret";
        let ciphertext = vault.encrypt("webhook:hook:signing", secret).unwrap();
        let url = format!("http://{address}/delivery");
        sqlx::query("INSERT INTO webhooks(id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state,signing_key_id,signing_ciphertext) VALUES('hook','server','channel','Hook','outgoing','hash',?,'owner','active',?,?)")
            .bind(&url).bind(vault.key_id()).bind(ciphertext).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO webhook_events(webhook_id,event_type) VALUES('hook','message_create')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','owner','owner','hello')")
            .execute(&pool).await.unwrap();
        let payload = serde_json::json!({"event_type":"message_create","channel_id":"channel","entity_id":"message","data":{"content":"hello"}})
            .to_string();
        sqlx::query("INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) VALUES('job','dedupe','webhook_delivery','delivery',1,'webhook:hook:1',?)")
            .bind(&payload).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO webhook_deliveries(id,webhook_id,external_job_id,delivery_id,event_type,event_version,payload_json) VALUES('row','hook','job','delivery','message_create',1,?)")
            .bind(&payload).execute(&pool).await.unwrap();
        let origin = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        let transport =
            crate::egress::ControlledHttpClient::scoped_origins(&[origin], 4096, 1).unwrap();
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
        assert_eq!(report.succeeded, 1);
        let request = receiver.await.unwrap();
        assert!(request.starts_with("POST /delivery HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-concord-delivery: delivery")
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-concord-signature-256: sha256=")
        );
        assert!(request.ends_with(&payload));
        let state: (String, String, i64) = sqlx::query_as("SELECT d.state,j.state,d.attempt_count FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(state, ("delivered".into(), "succeeded".into(), 1));
    }

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

    #[tokio::test]
    async fn canonical_channel_send_reaches_only_its_public_channel_webhook() {
        use crate::engine::messaging::{ContentFormat, MessagingService, SendMessageCommand};
        use crate::engine::permissions::DEFAULT_EVERYONE;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0_u8; 2048];
                let read = socket.read(&mut buffer).await.unwrap();
                bytes.extend_from_slice(&buffer[..read]);
                let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= header_end + 4 + length {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('owner','owner','canonical_id')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
            .bind(DEFAULT_EVERYONE.bits() as i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,is_private) VALUES('channel','server','#general',0),('sibling','server','#sibling',0),('private','server','#private',1)")
            .execute(&pool).await.unwrap();
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([41_u8; 32])).unwrap();
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
        for (id, channel) in [
            ("hook", "channel"),
            ("sibling-hook", "sibling"),
            ("private-hook", "private"),
        ] {
            let ciphertext = vault
                .encrypt(&format!("webhook:{id}:signing"), b"secret")
                .unwrap();
            sqlx::query("INSERT INTO webhooks(id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state,signing_key_id,signing_ciphertext) VALUES(?,'server',?,'Hook','outgoing',?,?,'owner','active',?,?)")
                .bind(id).bind(channel).bind(format!("hash-{id}")).bind(format!("http://{address}/{id}"))
                .bind(vault.key_id()).bind(ciphertext).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO webhook_events(id,webhook_id,event_type) VALUES(?,?,'message_create')",
            )
            .bind(format!("subscription-{id}"))
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("owner").await.unwrap().1;
        MessagingService::new(pool.clone(), auth, 4000)
            .send_channel_message(
                &actor,
                SendMessageCommand {
                    request_id: "request",
                    client_message_id: "client-message",
                    operation_generation: None,
                    conversation_id: None,
                    server_id: "server",
                    channel: "#general",
                    content: "canonical payload",
                    content_format: ContentFormat::Markdown,
                    reply_to_id: None,
                    attachment_ids: &[],
                    mentions: &[],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM webhook_deliveries")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        let origin = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        let transport =
            crate::egress::ControlledHttpClient::scoped_origins(&[origin], 8192, 1).unwrap();
        let dispatcher = WebhookDispatcher::new(pool.clone(), transport, vault, 8);
        let report = crate::jobs::run_once_matching(
            &pool,
            "worker",
            &dispatcher,
            &crate::jobs::JobSelection {
                operation_types: &["webhook_delivery"],
                lease_seconds: 30,
                limit: 10,
                max_attempts: 8,
            },
        )
        .await
        .unwrap();
        assert_eq!(report.succeeded, 1);
        let request = receiver.await.unwrap();
        assert!(request.starts_with("POST /hook HTTP/1.1"));
        assert!(request.contains("canonical payload"));
    }

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

    #[tokio::test]
    async fn grant_revoked_while_waiting_for_egress_admission_sends_no_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (pool, vault, transport) = queued_delivery_fixture(address).await;
        let blocker_transport = transport.clone();
        let blocker_url = reqwest::Url::parse(&format!("http://{address}/block")).unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = socket.read(&mut request).await.unwrap();
            accepted_tx.send(()).unwrap();
            release_rx.await.unwrap();
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_millis(150), listener.accept()).await
        });
        let blocker = tokio::spawn(async move {
            let request = blocker_transport
                .request(
                    reqwest::Method::GET,
                    blocker_url,
                    crate::egress::RedirectPolicy::Reject,
                )
                .unwrap();
            blocker_transport.send(request).await.unwrap();
        });
        accepted_rx.await.unwrap();
        let dispatcher = WebhookDispatcher::new(pool.clone(), transport, vault, 8);
        let worker_pool = pool.clone();
        let worker = tokio::spawn(async move {
            crate::jobs::run_once_matching(
                &worker_pool,
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
            .unwrap()
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        sqlx::query("UPDATE webhooks SET credential_state='revoked',revoked_at=datetime('now'),grant_version=grant_version+1 WHERE id='hook'")
            .execute(&pool).await.unwrap();
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        let report = worker.await.unwrap();
        assert_eq!(report.retried_or_failed, 1);
        assert!(
            server.await.unwrap().is_err(),
            "revoked delivery reached the network"
        );
    }
}
