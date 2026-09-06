use super::*;
use crate::db::pool::{create_pool, run_migrations};
use std::io::Write;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn setup_db() -> SqlitePool {
    let pool = create_pool(":memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

async fn create_test_user(pool: &SqlitePool, user_id: &str, username: &str) {
    sqlx::query("INSERT INTO users (id, username) VALUES (?, ?)")
        .bind(user_id)
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_sync_enabled_default_false() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    let enabled = is_sync_enabled(&pool, "u1").await.unwrap();
    assert!(!enabled);
}

#[tokio::test]
async fn test_set_sync_enabled_toggle() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;

    set_sync_enabled(&pool, "u1", true).await.unwrap();
    assert!(is_sync_enabled(&pool, "u1").await.unwrap());

    set_sync_enabled(&pool, "u1", false).await.unwrap();
    assert!(!is_sync_enabled(&pool, "u1").await.unwrap());
}

#[tokio::test]
async fn test_concord_message_record_serialization() {
    let record = ConcordMessageRecord {
        record_type: "chat.concord.message",
        text: "Hello world",
        server_id: "s1",
        channel_name: "#general",
        created_at: "2026-02-11T00:00:00Z",
        reply_to: None,
    };
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["$type"], "chat.concord.message");
    assert_eq!(json["text"], "Hello world");
    assert_eq!(json["serverId"], "s1");
    assert_eq!(json["channelName"], "#general");
    assert!(json.get("replyTo").is_none());
}

#[tokio::test]
async fn test_concord_message_record_with_reply() {
    let record = ConcordMessageRecord {
        record_type: "chat.concord.message",
        text: "Reply",
        server_id: "s1",
        channel_name: "#general",
        created_at: "2026-02-11T00:00:00Z",
        reply_to: Some("at://did:plc:abc/chat.concord.message/xyz"),
    };
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["replyTo"], "at://did:plc:abc/chat.concord.message/xyz");
}

#[tokio::test]
async fn dispatcher_cancels_revoked_grant_without_contacting_provider() {
    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    for statement in [
        "INSERT INTO servers(id,name,owner_id) VALUES('s1','Test','u1')",
        "INSERT INTO server_members(server_id,user_id,role) VALUES('s1','u1','owner')",
        "INSERT INTO channels(id,server_id,name,atproto_publication_enabled) VALUES('c1','s1','#public',1)",
        "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled) VALUES('u1','c1',1)",
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('m1','s1','c1','u1','alice','public')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("u1").await.unwrap().1;
    let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
    let admission = crate::engine::write_admission::WriteAdmission::new(pool.clone());
    let publication = crate::db::queries::atproto::request_publication(
        &admission,
        &authorization,
        &auth,
        &actor,
        "m1",
    )
    .await
    .unwrap();
    sqlx::query("UPDATE atproto_publication_grants SET enabled=0,grant_version=grant_version+1 WHERE user_id='u1' AND channel_id='c1'")
        .execute(&pool).await.unwrap();

    let mut key_file = tempfile::NamedTempFile::new().unwrap();
    writeln!(key_file, "{}", hex::encode([4_u8; 32])).unwrap();
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
    let dispatcher = AtprotoPublicationDispatcher::new(
        pool.clone(),
        crate::egress::ControlledHttpClient::fixture("127.0.0.1:9".parse().unwrap(), 1024),
        vault,
        std::sync::Arc::new(
            atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private)
                .unwrap(),
        ),
        "https://client.example/metadata".into(),
        "https://client.example/callback".into(),
    );
    let report = crate::jobs::run_once(&pool, "at-worker", &dispatcher, 30, 1, 3)
        .await
        .unwrap();
    assert_eq!(report.succeeded, 1);
    let state: String = sqlx::query_scalar("SELECT status FROM atproto_publications WHERE id=?")
        .bind(publication.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "cancelled");
}

#[tokio::test]
async fn uncertain_put_update_and_delete_reconcile_one_stable_remote_record() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let observed = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let server_observed = observed.clone();
    let server = tokio::spawn(async move {
        for attempt in 0..6 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + content_length {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap();
            server_observed.lock().await.push(body);
            if attempt % 2 == 0 {
                // The provider applied the mutation, then its response was lost.
                continue;
            }
            let (status, response) = if attempt == 5 {
                ("400 Bad Request", r#"{"error":"RecordNotFound"}"#)
            } else {
                (
                    "200 OK",
                    r#"{"uri":"at://did:plc:u1/app.bsky.feed.post/stable","cid":"cid"}"#,
                )
            };
            socket.write_all(format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            ).as_bytes()).await.unwrap();
        }
    });

    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    sqlx::query("INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES('oa','u1','atproto','did:plc:u1')")
        .execute(&pool).await.unwrap();
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([41_u8; 32])).unwrap();
    let vault = crate::secrets::SecretVault::load(key_file.path()).unwrap();
    let dpop_key =
        atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private).unwrap();
    let credentials = crate::db::queries::users::AtprotoCredentials {
        did: "did:plc:u1".into(),
        access_token: "access".into(),
        refresh_token: String::new(),
        dpop_private_key: serde_json::to_string(&atproto_oauth::jwk::generate(&dpop_key).unwrap())
            .unwrap(),
        pds_url: "http://fixture.test".into(),
        authorization_issuer: String::new(),
        token_endpoint: String::new(),
        token_expires_at: "2999-01-01T00:00:00Z".into(),
        credential_version: 0,
    };
    crate::db::queries::users::store_atproto_credentials_encrypted(
        &pool,
        &vault,
        "u1",
        &credentials,
    )
    .await
    .unwrap();
    let transport = crate::egress::ControlledHttpClient::fixture(address, 16_384);
    let signing_key =
        atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private).unwrap();
    let session = pds_client::PdsSession {
        transport: &transport,
        pool: &pool,
        vault: &vault,
        user_id: "u1",
        signing_key: &signing_key,
        client_id: "https://client.example",
        redirect_uri: "https://client.example/callback",
    };
    let first = serde_json::json!({"$type":"app.bsky.feed.post","text":"first"});
    assert!(
        pds_client::put_record(&session, "app.bsky.feed.post", "stable", &first)
            .await
            .is_err()
    );
    pds_client::put_record(&session, "app.bsky.feed.post", "stable", &first)
        .await
        .unwrap();
    let edited = serde_json::json!({"$type":"app.bsky.feed.post","text":"edited"});
    assert!(
        pds_client::put_record(&session, "app.bsky.feed.post", "stable", &edited)
            .await
            .is_err()
    );
    pds_client::put_record(&session, "app.bsky.feed.post", "stable", &edited)
        .await
        .unwrap();
    assert!(
        delete_record_from_pds(&session, "app.bsky.feed.post", "stable")
            .await
            .is_err()
    );
    delete_record_from_pds(&session, "app.bsky.feed.post", "stable")
        .await
        .unwrap();
    server.await.unwrap();
    let requests = observed.lock().await;
    assert_eq!(requests.len(), 6);
    for pair in requests.chunks_exact(2) {
        assert_eq!(pair[0]["rkey"], "stable");
        assert_eq!(pair[0], pair[1]);
    }
}

#[tokio::test]
async fn lost_create_response_then_source_delete_removes_the_stable_remote_record() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let observed = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_observed = observed.clone();
    let server = tokio::spawn(async move {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let path = headers
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap()
                .to_owned();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + content_length {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap();
            server_observed.lock().await.push((path, body));
            if attempt == 0 {
                // The provider committed putRecord, then the response was lost.
                continue;
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .await
                .unwrap();
        }
    });

    let pool = setup_db().await;
    create_test_user(&pool, "u1", "alice").await;
    for statement in [
        "INSERT INTO servers(id,name,owner_id) VALUES('s1','Test','u1')",
        "INSERT INTO server_members(server_id,user_id,role) VALUES('s1','u1','owner')",
        "INSERT INTO channels(id,server_id,name,atproto_publication_enabled) VALUES('c1','s1','#public',1)",
        "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled) VALUES('u1','c1',1)",
        "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('m1','s1','c1','u1','alice','public')",
        "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) VALUES('oa','u1','atproto','did:plc:u1')",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
    let actor = auth.issue_web_session("u1").await.unwrap().1;
    let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
    let admission = crate::engine::write_admission::WriteAdmission::new(pool.clone());
    let publication = crate::db::queries::atproto::request_publication(
        &admission,
        &authorization,
        &auth,
        &actor,
        "m1",
    )
    .await
    .unwrap();

    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), hex::encode([41_u8; 32])).unwrap();
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
    let dpop_key =
        atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private).unwrap();
    crate::db::queries::users::store_atproto_credentials_encrypted(
        &pool,
        &vault,
        "u1",
        &crate::db::queries::users::AtprotoCredentials {
            did: "did:plc:u1".into(),
            access_token: "access".into(),
            refresh_token: String::new(),
            dpop_private_key: serde_json::to_string(
                &atproto_oauth::jwk::generate(&dpop_key).unwrap(),
            )
            .unwrap(),
            pds_url: "http://fixture.test".into(),
            authorization_issuer: String::new(),
            token_endpoint: String::new(),
            token_expires_at: "2999-01-01T00:00:00Z".into(),
            credential_version: 0,
        },
    )
    .await
    .unwrap();
    let dispatcher = AtprotoPublicationDispatcher::new(
        pool.clone(),
        crate::egress::ControlledHttpClient::fixture(address, 16_384),
        vault,
        std::sync::Arc::new(
            atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private)
                .unwrap(),
        ),
        "https://client.example".into(),
        "https://client.example/callback".into(),
    );
    let first = crate::jobs::run_once(&pool, "at-worker", &dispatcher, 30, 1, 1)
        .await
        .unwrap();
    assert_eq!(first.retried_or_failed, 1);

    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("UPDATE messages SET deleted_at=datetime('now'),entity_version=2 WHERE id='m1'")
        .execute(&mut *transaction)
        .await
        .unwrap();
    crate::db::queries::atproto::schedule_source_mutation(&mut transaction, "m1", 2, true)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let second = crate::jobs::run_once(&pool, "at-worker", &dispatcher, 30, 1, 1)
        .await
        .unwrap();
    assert_eq!(second.succeeded, 1);
    server.await.unwrap();

    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM atproto_publications WHERE id=?",)
            .bind(&publication.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "deleted"
    );
    let requests = observed.lock().await;
    assert!(requests[0].0.contains("com.atproto.repo.putRecord"));
    assert!(requests[1].0.contains("com.atproto.repo.deleteRecord"));
    assert_eq!(requests[0].1["rkey"], requests[1].1["rkey"]);
}
