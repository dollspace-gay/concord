//! AT Protocol custom lexicon record management.
//!
//! Defines Concord-specific AT Protocol lexicons and provides functions
//! to sync messages to a user's PDS as custom records.
//!
//! ## Custom Lexicons
//!
//! - `chat.concord.message` — A chat message with text, server, and channel context.
//!   Fields: text, serverId, channelName, createdAt, replyTo (optional)
//!
//! - `chat.concord.server` — Server membership record.
//!   Fields: serverName, serverDescription, joinedAt
//!
//! - `chat.concord.channel` — Channel subscription record.
//!   Fields: channelName, serverId, topic

use anyhow::Result;
use atproto_identity::key::KeyData;
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::warn;

use super::pds_client::{self, CreateRecordResponse};

#[derive(Clone)]
pub struct AtprotoPublicationDispatcher {
    pool: SqlitePool,
    authorization: crate::engine::authorization::AuthorizationService,
    transport: crate::egress::ControlledHttpClient,
    vault: std::sync::Arc<crate::secrets::SecretVault>,
    signing_key: std::sync::Arc<KeyData>,
    client_id: String,
    redirect_uri: String,
}

impl AtprotoPublicationDispatcher {
    pub fn new(
        pool: SqlitePool,
        transport: crate::egress::ControlledHttpClient,
        vault: std::sync::Arc<crate::secrets::SecretVault>,
        signing_key: std::sync::Arc<KeyData>,
        client_id: String,
        redirect_uri: String,
    ) -> Self {
        Self {
            authorization: crate::engine::authorization::AuthorizationService::new(pool.clone()),
            pool,
            transport,
            vault,
            signing_key,
            client_id,
            redirect_uri,
        }
    }

    async fn dispatch_publication(
        &self,
        job: &crate::jobs::ClaimedJob,
    ) -> Result<(), crate::jobs::DispatchFailure> {
        if !matches!(
            job.operation_type.as_str(),
            "atproto_publish" | "atproto_update" | "atproto_delete"
        ) {
            return Err(permanent_failure("unsupported_operation"));
        }
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                i64,
                String,
                String,
                String,
                Option<String>,
                String,
                String,
                Option<i64>,
                i64,
            ),
        >(
            "SELECT p.user_id,p.source_message_id,p.source_version,p.status,p.collection,
                    p.record_key,p.remote_uri,m.content,m.created_at,g.grant_version,
                    c.atproto_publication_enabled
             FROM atproto_publications p
             JOIN messages m ON m.id=p.source_message_id
             JOIN channels c ON c.id=m.channel_id
             LEFT JOIN atproto_publication_grants g
               ON g.user_id=p.user_id AND g.channel_id=c.id AND g.enabled=1
             WHERE p.id=?",
        )
        .bind(&job.resource_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| retryable_failure("publication_db_unavailable"))?
        .ok_or_else(|| permanent_failure("publication_ineligible"))?;
        let (
            user_id,
            message_id,
            source_version,
            status,
            collection,
            record_key,
            _remote_uri,
            content,
            created_at,
            grant_version,
            channel_enabled,
        ) = row;
        if source_version != job.resource_version {
            return Ok(());
        }
        let expected_grant =
            grant_version.map(|version| format!("atproto-user:{user_id}:{version}"));
        let deleting = job.operation_type == "atproto_delete";
        if (deleting && status != "delete_pending")
            || (!deleting && !matches!(status.as_str(), "pending" | "update_pending"))
        {
            return Ok(());
        }
        if !deleting
            && (channel_enabled == 0
                || expected_grant.as_deref() != Some(job.destination_grant.as_str()))
        {
            sqlx::query("UPDATE atproto_publications SET status='cancelled',safe_error_code='grant_revoked',updated_at=datetime('now') WHERE id=? AND source_version=?")
                .bind(&job.resource_id).bind(source_version).execute(&self.pool).await
                .map_err(|_| retryable_failure("publication_db_unavailable"))?;
            return Ok(());
        }
        let channel_id: String = sqlx::query_scalar("SELECT channel_id FROM messages WHERE id=?")
            .bind(&message_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| retryable_failure("publication_db_unavailable"))?;
        if !deleting
            && (self
                .authorization
                .authorize_channel(
                    &user_id,
                    &channel_id,
                    crate::engine::authorization::ChannelAction::View,
                )
                .await
                .is_err()
                || self
                    .authorization
                    .authorize_channel(
                        &user_id,
                        &channel_id,
                        crate::engine::authorization::ChannelAction::ReadHistory,
                    )
                    .await
                    .is_err())
        {
            return Err(permanent_failure("publication_authorization_revoked"));
        }
        if !deleting {
            let still_eligible: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                   SELECT 1 FROM atproto_publications p
                   JOIN messages m ON m.id=p.source_message_id
                   JOIN channels c ON c.id=m.channel_id
                   JOIN atproto_publication_grants g
                     ON g.user_id=p.user_id AND g.channel_id=c.id AND g.enabled=1
                   WHERE p.id=? AND p.source_version=? AND m.entity_version=?
                     AND m.deleted_at IS NULL AND g.grant_version=?
                     AND c.atproto_publication_enabled=1 AND c.is_private=0
                     AND c.visibility_repair_required=0 AND c.parent_channel_id IS NULL
                     AND c.channel_type NOT IN ('public_thread','private_thread')
                 )",
            )
            .bind(&job.resource_id)
            .bind(source_version)
            .bind(source_version)
            .bind(grant_version)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| retryable_failure("publication_db_unavailable"))?;
            if !still_eligible {
                return Err(permanent_failure("publication_became_ineligible"));
            }
        }
        let session = pds_client::PdsSession {
            transport: &self.transport,
            pool: &self.pool,
            vault: &self.vault,
            user_id: &user_id,
            signing_key: &self.signing_key,
            client_id: &self.client_id,
            redirect_uri: &self.redirect_uri,
        };
        if deleting {
            delete_record_from_pds(&session, &collection, &record_key)
                .await
                .map_err(|_| retryable_failure("provider_unavailable"))?;
            sqlx::query("UPDATE atproto_publications SET status='deleted',updated_at=datetime('now'),safe_error_code=NULL WHERE id=? AND source_version=?")
                .bind(&job.resource_id).bind(source_version).execute(&self.pool).await
                .map_err(|_| retryable_failure("publication_db_unavailable"))?;
            return Ok(());
        }
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": content.chars().take(300).collect::<String>(),
            "createdAt": created_at,
        });
        let response = pds_client::put_record(&session, &collection, &record_key, &record)
            .await
            .map_err(|_| retryable_failure("provider_unavailable"))?;
        sqlx::query("UPDATE atproto_publications SET status='published',remote_uri=?,remote_cid=?,safe_error_code=NULL,updated_at=datetime('now') WHERE id=? AND source_version=?")
            .bind(response.uri).bind(response.cid).bind(&job.resource_id).bind(source_version)
            .execute(&self.pool).await.map_err(|_| retryable_failure("publication_db_unavailable"))?;
        Ok(())
    }
}

impl crate::jobs::JobDispatcher for AtprotoPublicationDispatcher {
    fn dispatch<'a>(
        &'a self,
        job: &'a crate::jobs::ClaimedJob,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), crate::jobs::DispatchFailure>> + Send + 'a>,
    > {
        Box::pin(self.dispatch_publication(job))
    }
}

fn retryable_failure(code: &'static str) -> crate::jobs::DispatchFailure {
    crate::jobs::DispatchFailure {
        safe_code: code,
        retry_after_seconds: None,
        permanent: false,
    }
}

fn permanent_failure(code: &'static str) -> crate::jobs::DispatchFailure {
    crate::jobs::DispatchFailure {
        safe_code: code,
        retry_after_seconds: None,
        permanent: true,
    }
}

/// A `chat.concord.message` record for syncing to AT Protocol PDS.
#[derive(Serialize)]
pub struct ConcordMessageRecord<'a> {
    #[serde(rename = "$type")]
    pub record_type: &'static str,
    pub text: &'a str,
    #[serde(rename = "serverId")]
    pub server_id: &'a str,
    #[serde(rename = "channelName")]
    pub channel_name: &'a str,
    #[serde(rename = "createdAt")]
    pub created_at: &'a str,
    #[serde(rename = "replyTo", skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<&'a str>,
}

/// Parameters for syncing a message to a user's PDS.
pub struct SyncMessageParams<'a> {
    pub transport: &'a crate::egress::ControlledHttpClient,
    pub pool: &'a SqlitePool,
    pub vault: &'a crate::secrets::SecretVault,
    pub user_id: &'a str,
    pub text: &'a str,
    pub server_id: &'a str,
    pub channel_name: &'a str,
    pub created_at: &'a str,
    pub reply_to: Option<&'a str>,
    pub signing_key: &'a KeyData,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
}

/// Sync a single message to the user's PDS as a `chat.concord.message` record.
///
/// Returns the AT-URI and CID of the created record.
/// Errors are non-fatal — callers should log but not block on failure.
pub async fn sync_message_to_pds(p: &SyncMessageParams<'_>) -> Result<CreateRecordResponse> {
    let record = ConcordMessageRecord {
        record_type: "chat.concord.message",
        text: p.text,
        server_id: p.server_id,
        channel_name: p.channel_name,
        created_at: p.created_at,
        reply_to: p.reply_to,
    };

    let session = pds_client::PdsSession {
        transport: p.transport,
        pool: p.pool,
        vault: p.vault,
        user_id: p.user_id,
        signing_key: p.signing_key,
        client_id: p.client_id,
        redirect_uri: p.redirect_uri,
    };
    pds_client::create_record(&session, "chat.concord.message", &record).await
}

/// Delete a synced message record from the user's PDS.
///
/// `record_key` is the rkey portion of the AT-URI (e.g., the last segment of
/// `at://did:plc:abc/chat.concord.message/rkey123`).
pub async fn delete_message_from_pds(
    session: &pds_client::PdsSession<'_>,
    record_key: &str,
) -> Result<()> {
    delete_record_from_pds(session, "chat.concord.message", record_key).await
}

pub async fn delete_record_from_pds(
    session: &pds_client::PdsSession<'_>,
    collection: &str,
    record_key: &str,
) -> Result<()> {
    let creds = crate::db::queries::users::get_atproto_credentials_encrypted(
        session.pool,
        session.vault,
        session.user_id,
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("No AT Protocol credentials for user"))?;

    let body = serde_json::json!({
        "repo": creds.did,
        "collection": collection,
        "rkey": record_key,
    });
    let body_json = serde_json::to_string(&body)?;

    let result = pds_client::pds_xrpc_call(&pds_client::PdsXrpcParams {
        session,
        method: "POST",
        xrpc_endpoint: "com.atproto.repo.deleteRecord",
        body: Some(body_json.as_bytes()),
        content_type: "application/json",
    })
    .await;

    match result {
        Ok(_) => Ok(()),
        Err(e)
            if e.downcast_ref::<pds_client::PdsRequestError>()
                .is_some_and(|error| {
                    matches!(error, pds_client::PdsRequestError::RemoteStatus { status, body }
                        if *status == reqwest::StatusCode::BAD_REQUEST
                        && serde_json::from_slice::<serde_json::Value>(body).ok()
                            .and_then(|value| value.get("error").and_then(|error| error.as_str()).map(str::to_owned))
                            .as_deref() == Some("RecordNotFound"))
                }) =>
        {
            Ok(())
        }
        Err(e) => {
            warn!(error = %e, user_id=session.user_id, record_key, "Failed to delete AT Protocol record");
            Err(e)
        }
    }
}

/// Check if a user has AT Protocol record sync enabled.
pub async fn is_sync_enabled(pool: &SqlitePool, user_id: &str) -> Result<bool, sqlx::Error> {
    let val: Option<i64> =
        sqlx::query_scalar("SELECT atproto_sync_enabled FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(val.unwrap_or(0) != 0)
}

/// Set the AT Protocol record sync preference for a user.
pub async fn set_sync_enabled(
    pool: &SqlitePool,
    user_id: &str,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET atproto_sync_enabled = ? WHERE id = ?")
        .bind(if enabled { 1i64 } else { 0i64 })
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
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
        let state: String =
            sqlx::query_scalar("SELECT status FROM atproto_publications WHERE id=?")
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
        let observed =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
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
                    serde_json::from_slice(&bytes[header_end..header_end + content_length])
                        .unwrap();
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
            atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private)
                .unwrap();
        let credentials = crate::db::queries::users::AtprotoCredentials {
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
            atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private)
                .unwrap();
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
                    serde_json::from_slice(&bytes[header_end..header_end + content_length])
                        .unwrap();
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
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
        let dpop_key =
            atproto_identity::key::generate_key(atproto_identity::key::KeyType::P256Private)
                .unwrap();
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
        sqlx::query(
            "UPDATE messages SET deleted_at=datetime('now'),entity_version=2 WHERE id='m1'",
        )
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
}
