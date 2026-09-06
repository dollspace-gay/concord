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
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('message','server','channel','owner','owner','secret-at-queue-time')")
        .execute(&pool).await.unwrap();
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([43_u8; 32])).unwrap();
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
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
            .write_all(format!("HTTP/1.1 {status} Failure\r\nContent-Length: 0\r\n\r\n").as_bytes())
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
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')")
        .execute(&pool)
        .await
        .unwrap();
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([31_u8; 32])).unwrap();
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
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

mod behavior;
mod lifecycle;
mod messaging;
mod recovery;
mod revocation;
mod validation;
