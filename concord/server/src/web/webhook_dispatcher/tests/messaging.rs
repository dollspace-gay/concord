use super::*;

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
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('channel','server','#channel')")
        .execute(&pool)
        .await
        .unwrap();
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([29_u8; 32])).unwrap();
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
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
    sqlx::query(
        "INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('owner','owner','canonical_id')",
    )
    .execute(&pool)
    .await
    .unwrap();
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
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
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
