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
    pub pool: &'a SqlitePool,
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

    pds_client::create_record(
        p.pool,
        p.user_id,
        "chat.concord.message",
        &record,
        p.signing_key,
        p.client_id,
        p.redirect_uri,
    )
    .await
}

/// Delete a synced message record from the user's PDS.
///
/// `record_key` is the rkey portion of the AT-URI (e.g., the last segment of
/// `at://did:plc:abc/chat.concord.message/rkey123`).
pub async fn delete_message_from_pds(
    pool: &SqlitePool,
    user_id: &str,
    record_key: &str,
    signing_key: &KeyData,
    client_id: &str,
    redirect_uri: &str,
) -> Result<()> {
    let creds = crate::db::queries::users::get_atproto_credentials(pool, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No AT Protocol credentials for user"))?;

    let body = serde_json::json!({
        "repo": creds.did,
        "collection": "chat.concord.message",
        "rkey": record_key,
    });
    let body_json = serde_json::to_string(&body)?;

    let result = pds_client::pds_xrpc_call(&pds_client::PdsXrpcParams {
        pool,
        user_id,
        method: "POST",
        xrpc_endpoint: "com.atproto.repo.deleteRecord",
        body: Some(body_json.as_bytes()),
        content_type: "application/json",
        signing_key,
        client_id,
        redirect_uri,
    })
    .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            warn!(error = %e, user_id, record_key, "Failed to delete AT Protocol record");
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
}
