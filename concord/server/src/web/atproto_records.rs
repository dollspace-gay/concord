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
mod tests;
