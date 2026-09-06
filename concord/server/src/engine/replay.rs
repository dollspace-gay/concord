use std::fmt;

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use chrono::Utc;
use rand::RngCore;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::auth::authority::{Actor, AuthService};
use crate::engine::authorization::{AuthorizationError, AuthorizationService, ConversationAction};
use crate::engine::ids::ConversationId;

use crate::contract::PROTOCOL_VERSION;
const CURSOR_LIFETIME_SECONDS: i64 = 7 * 24 * 60 * 60;
const MAX_SUBSCRIPTIONS: usize = 100;
const MAX_REPLAY_EVENTS: usize = 100;
const MAX_SNAPSHOT_MESSAGES: usize = 100;
const MAX_SNAPSHOT_REACTION_GROUPS: usize = 2_000;
const MAX_SNAPSHOT_PROJECTION_BYTES: usize = 768 * 1024;
const MAX_CURSOR_BYTES: usize = 4096;

#[derive(Clone)]
pub struct ReplayService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    write_admission: super::write_admission::WriteAdmission,
    cursor_cipher: XChaCha20Poly1305,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DurableMessageProjection {
    pub message_id: String,
    pub conversation_id: ConversationId,
    pub sequence: String,
    pub entity_version: u64,
    pub sender_id: String,
    pub sender_nick: String,
    pub content: Option<String>,
    pub content_format: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted: bool,
    pub reply_to_id: Option<String>,
    pub reply_to: Option<DurableReplyProjection>,
    pub attachments: Vec<DurableAttachmentProjection>,
    pub mentions: Vec<crate::engine::messaging::MessageMention>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_embeds: Option<Vec<crate::engine::events::RichEmbedInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<crate::engine::events::MessageComponent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DurableReplyProjection {
    pub message_id: String,
    pub sender_id: String,
    pub sender_nick: String,
    pub content: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DurableAttachmentProjection {
    pub attachment_id: String,
    pub filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub state_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DurableEventProjection {
    pub kind: String,
    pub conversation_id: ConversationId,
    pub entity_type: String,
    pub entity_id: String,
    pub entity_version: u64,
    pub message: Option<DurableMessageProjection>,
    pub reaction: Option<DurableReactionProjection>,
    pub read_state: Option<DurableReadProjection>,
    pub descriptor: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DurableReactionProjection {
    pub message_id: String,
    pub user_id: String,
    pub emoji: String,
    pub present: bool,
    pub entity_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SnapshotReactionGroup {
    pub message_id: String,
    pub emoji: String,
    pub count: u64,
    pub reacted_by_me: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DurableReadProjection {
    pub conversation_id: ConversationId,
    pub message_id: String,
    pub sequence: String,
    pub entity_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SyncSnapshot {
    pub protocol_version: u32,
    pub operation_generation: String,
    pub cursor: String,
    pub messages: Vec<DurableMessageProjection>,
    pub reactions: Vec<SnapshotReactionGroup>,
    pub read_states: Vec<DurableReadProjection>,
    pub history_before: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ReplayBatch {
    pub protocol_version: u32,
    pub operation_generation: String,
    pub cursor: String,
    pub events: Vec<DurableEventProjection>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    CursorExpired,
    DatabaseRestored,
    CredentialChanged,
    SubscriptionChanged,
    AccessRevoked,
    ProtocolChanged,
    InvalidCursor,
}

#[derive(Debug)]
pub enum ReplayError {
    ResyncRequired(ResyncReason),
    Unavailable,
    InvalidInput,
    SnapshotTooLarge,
    DependencyUnavailable,
    Database(sqlx::Error),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResyncRequired(reason) => {
                write!(formatter, "resynchronization required: {reason:?}")
            }
            Self::Unavailable => formatter.write_str("resource unavailable"),
            Self::InvalidInput => formatter.write_str("invalid replay request"),
            Self::SnapshotTooLarge => {
                formatter.write_str("snapshot exceeds response budget; request fewer messages")
            }
            Self::DependencyUnavailable => formatter.write_str("replay dependency unavailable"),
            Self::Database(_) => formatter.write_str("replay storage unavailable"),
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for ReplayError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorClaims {
    protocol_version: u32,
    database_generation: String,
    principal_id: String,
    credential_id: String,
    credential_version: i64,
    subscription_hash: String,
    event_sequence: i64,
    expires_at: i64,
}

impl ReplayService {
    pub fn new(pool: SqlitePool, auth: AuthService, persistent_secret: &str) -> Self {
        let write_admission = super::write_admission::WriteAdmission::new(pool.clone());
        Self::new_with_write_admission(pool, auth, persistent_secret, write_admission)
    }

    pub(crate) fn new_with_write_admission(
        pool: SqlitePool,
        auth: AuthService,
        persistent_secret: &str,
        write_admission: super::write_admission::WriteAdmission,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"concord-replay-cursor-v1\0");
        hasher.update(persistent_secret.as_bytes());
        let key_bytes = hasher.finalize();
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            pool,
            auth,
            write_admission,
            cursor_cipher: XChaCha20Poly1305::new(Key::from_slice(&key_bytes)),
        }
    }

    pub async fn snapshot(
        &self,
        actor: &Actor,
        subscriptions: &[String],
    ) -> Result<SyncSnapshot, ReplayError> {
        self.snapshot_with_limit(actor, subscriptions, MAX_SNAPSHOT_MESSAGES)
            .await
    }

    pub async fn snapshot_with_limit(
        &self,
        actor: &Actor,
        subscriptions: &[String],
        message_limit: usize,
    ) -> Result<SyncSnapshot, ReplayError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Replay);
        let subscriptions = canonical_subscriptions(subscriptions)?;
        let message_limit = message_limit.clamp(1, MAX_SNAPSHOT_MESSAGES);
        self.auth
            .validate_actor(actor)
            .await
            .map_err(map_auth_error)?;
        let operation_generation = self
            .write_admission
            .current_operation_generation()
            .await
            .map_err(|_| ReplayError::Unavailable)?;
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        for conversation_id in &subscriptions {
            authorize_conversation(
                &self.authorization,
                &self.auth,
                &mut transaction,
                actor,
                conversation_id.as_str(),
            )
            .await?;
        }
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let high_water: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(event_sequence),0) FROM event_log")
                .fetch_one(&mut *transaction)
                .await?;
        let read_states =
            load_snapshot_reads(&mut transaction, actor.user_id().as_str(), &subscriptions).await?;
        let mut effective_limit = message_limit;
        let (messages, reactions) = loop {
            let messages =
                load_snapshot_messages(&mut transaction, &subscriptions, effective_limit).await?;
            let reactions = match load_snapshot_reactions(
                &mut transaction,
                &messages,
                actor.user_id().as_str(),
            )
            .await
            {
                Ok(reactions) => reactions,
                Err(ReplayError::SnapshotTooLarge) if effective_limit > 1 => {
                    effective_limit = (effective_limit / 2).max(1);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let projected_bytes = serde_json::to_vec(&(&messages, &reactions, &read_states))
                .map_err(|_| ReplayError::InvalidInput)?
                .len();
            if projected_bytes <= MAX_SNAPSHOT_PROJECTION_BYTES {
                break (messages, reactions);
            }
            if effective_limit == 1 {
                return Err(ReplayError::SnapshotTooLarge);
            }
            effective_limit = (effective_limit / 2).max(1);
        };
        let history_before = load_history_boundaries(&mut transaction, &messages).await?;
        transaction.commit().await?;
        let cursor = self.encode_cursor(actor, &subscriptions, &generation, high_water)?;
        let snapshot = SyncSnapshot {
            protocol_version: PROTOCOL_VERSION,
            operation_generation,
            cursor,
            messages,
            reactions,
            read_states,
            history_before,
        };
        metric.succeed();
        Ok(snapshot)
    }

    pub async fn replay(
        &self,
        actor: &Actor,
        subscriptions: &[String],
        cursor: &str,
        limit: usize,
    ) -> Result<ReplayBatch, ReplayError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Replay);
        let subscriptions = canonical_subscriptions(subscriptions)?;
        let claims = self.decode_cursor(cursor)?;
        self.validate_cursor_actor(&claims, actor, &subscriptions)?;
        self.auth
            .validate_actor(actor)
            .await
            .map_err(map_auth_error)?;
        let operation_generation = self
            .write_admission
            .current_operation_generation()
            .await
            .map_err(|_| ReplayError::Unavailable)?;
        let limit = limit.clamp(1, MAX_REPLAY_EVENTS);
        let mut transaction = self.pool.begin().await?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        if generation != claims.database_generation {
            return Err(ReplayError::ResyncRequired(ResyncReason::DatabaseRestored));
        }
        for conversation_id in &subscriptions {
            match authorize_conversation(
                &self.authorization,
                &self.auth,
                &mut transaction,
                actor,
                conversation_id.as_str(),
            )
            .await
            {
                Ok(()) => {}
                Err(ReplayError::Unavailable) => {
                    return Err(ReplayError::ResyncRequired(ResyncReason::AccessRevoked));
                }
                Err(error) => return Err(error),
            }
        }
        let retained_from: i64 = sqlx::query_scalar(
            "SELECT retained_from_sequence FROM event_retention_state WHERE singleton=1",
        )
        .fetch_one(&mut *transaction)
        .await?;
        // A cursor stores the last consumed sequence. It is replayable when the next
        // sequence is still retained, including retained_from - 1 at the boundary.
        if claims.event_sequence.saturating_add(1) < retained_from {
            return Err(ReplayError::ResyncRequired(ResyncReason::CursorExpired));
        }
        let global_high_water: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(event_sequence),0) FROM event_log")
                .fetch_one(&mut *transaction)
                .await?;
        let rows = if subscriptions.is_empty() {
            Vec::new()
        } else {
            let mut builder = sqlx::QueryBuilder::new(
                "SELECT event_sequence,conversation_id,event_kind,entity_type,entity_id, \
                        entity_version,descriptor_json FROM event_log \
                 WHERE database_generation=",
            );
            builder.push_bind(&generation);
            builder.push(" AND event_sequence>");
            builder.push_bind(claims.event_sequence);
            builder.push(" AND conversation_id IN (");
            let mut separated = builder.separated(",");
            for subscription in &subscriptions {
                separated.push_bind(subscription.as_str());
            }
            separated.push_unseparated(
                ") AND (entity_type<>'read_state' OR json_extract(descriptor_json,'$.user_id')=",
            );
            builder.push_bind(actor.user_id().as_str());
            builder.push(") ORDER BY event_sequence LIMIT ");
            builder.push_bind((limit + 1) as i64);
            builder.build().fetch_all(&mut *transaction).await?
        };
        let mut scanned_high_water = claims.event_sequence;
        let mut events = Vec::new();
        let has_more = rows.len() > limit;
        for row in rows.into_iter().take(limit) {
            let event_sequence: i64 = row.get(0);
            scanned_high_water = event_sequence;
            let Some(conversation_id) = row.get::<Option<String>, _>(1) else {
                continue;
            };
            let event_kind: String = row.get(2);
            let entity_type: String = row.get(3);
            let entity_id: String = row.get(4);
            let entity_version: i64 = row.get(5);
            let mut descriptor: serde_json::Value = serde_json::from_str(row.get::<&str, _>(6))
                .map_err(|_| ReplayError::InvalidInput)?;
            if entity_type == "read_state"
                && descriptor
                    .get("user_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(actor.user_id().as_str())
            {
                continue;
            }
            let message = if entity_type == "message" {
                load_message_projection(&mut transaction, &entity_id).await?
            } else {
                None
            };
            let reaction = if entity_type == "reaction" {
                load_reaction_projection(&mut transaction, &entity_id, &descriptor).await?
            } else {
                None
            };
            let read_state = if entity_type == "read_state" {
                load_read_projection(
                    &mut transaction,
                    actor.user_id().as_str(),
                    &conversation_id,
                    &entity_id,
                )
                .await?
            } else {
                None
            };
            let current_entity_version = resolve_current_event_state(
                &mut transaction,
                &entity_type,
                &entity_id,
                entity_version,
                &mut descriptor,
            )
            .await?;
            let conversation_id = ConversationId::from_stored(conversation_id)
                .map_err(|_| ReplayError::InvalidInput)?;
            events.push(DurableEventProjection {
                kind: event_kind,
                conversation_id,
                entity_type,
                entity_id,
                entity_version: current_entity_version as u64,
                message,
                reaction,
                read_state,
                descriptor,
            });
        }
        if !has_more {
            scanned_high_water = global_high_water;
        }
        transaction.commit().await?;
        let next_cursor =
            self.encode_cursor(actor, &subscriptions, &generation, scanned_high_water)?;
        let batch = ReplayBatch {
            protocol_version: PROTOCOL_VERSION,
            operation_generation,
            cursor: next_cursor,
            events,
            has_more,
        };
        metric.succeed();
        Ok(batch)
    }

    /// Resolve one durable descriptor to current state under current authority.
    /// `None` means the event is no longer visible to this principal.
    pub async fn project_event(
        &self,
        actor: &Actor,
        event_sequence: i64,
    ) -> Result<Option<(ConversationId, DurableEventProjection)>, ReplayError> {
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let Some(row) = sqlx::query(
            "SELECT conversation_id,event_kind,entity_type,entity_id,entity_version,descriptor_json \
             FROM event_log WHERE event_sequence=?",
        )
        .bind(event_sequence)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            return Ok(None);
        };
        let Some(conversation_id) = row.get::<Option<String>, _>(0) else {
            return Ok(None);
        };
        match authorize_conversation(
            &self.authorization,
            &self.auth,
            &mut transaction,
            actor,
            &conversation_id,
        )
        .await
        {
            Ok(()) => {}
            Err(ReplayError::Unavailable) => return Ok(None),
            Err(error) => return Err(error),
        }
        let event_kind: String = row.get(1);
        let entity_type: String = row.get(2);
        let entity_id: String = row.get(3);
        let recorded_version: i64 = row.get(4);
        let mut descriptor: serde_json::Value =
            serde_json::from_str(row.get::<&str, _>(5)).map_err(|_| ReplayError::InvalidInput)?;
        if entity_type == "read_state"
            && descriptor
                .get("user_id")
                .and_then(serde_json::Value::as_str)
                != Some(actor.user_id().as_str())
        {
            return Ok(None);
        }
        let message = if entity_type == "message" {
            load_message_projection(&mut transaction, &entity_id).await?
        } else {
            None
        };
        let reaction = if entity_type == "reaction" {
            load_reaction_projection(&mut transaction, &entity_id, &descriptor).await?
        } else {
            None
        };
        let read_state = if entity_type == "read_state" {
            load_read_projection(
                &mut transaction,
                actor.user_id().as_str(),
                &conversation_id,
                &entity_id,
            )
            .await?
        } else {
            None
        };
        let entity_version = resolve_current_event_state(
            &mut transaction,
            &entity_type,
            &entity_id,
            recorded_version,
            &mut descriptor,
        )
        .await?;
        transaction.commit().await?;
        let conversation_id =
            ConversationId::from_stored(conversation_id).map_err(|_| ReplayError::InvalidInput)?;
        Ok(Some((
            conversation_id.clone(),
            DurableEventProjection {
                kind: event_kind,
                conversation_id,
                entity_type,
                entity_id,
                entity_version: entity_version as u64,
                message,
                reaction,
                read_state,
                descriptor,
            },
        )))
    }

    fn encode_cursor(
        &self,
        actor: &Actor,
        subscriptions: &[ConversationId],
        generation: &str,
        event_sequence: i64,
    ) -> Result<String, ReplayError> {
        let now = Utc::now().timestamp();
        let expires_at = actor
            .expires_at()
            .unwrap_or(now + CURSOR_LIFETIME_SECONDS)
            .min(now + CURSOR_LIFETIME_SECONDS);
        let claims = CursorClaims {
            protocol_version: PROTOCOL_VERSION,
            database_generation: generation.to_owned(),
            principal_id: actor.user_id().as_str().to_owned(),
            credential_id: actor.credential_id().as_str().to_owned(),
            credential_version: actor.credential_version(),
            subscription_hash: subscription_hash(subscriptions),
            event_sequence,
            expires_at,
        };
        let plaintext = serde_json::to_vec(&claims).map_err(|_| ReplayError::InvalidInput)?;
        let mut nonce = [0_u8; 24];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .cursor_cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|_| ReplayError::InvalidInput)?;
        let mut encoded = nonce.to_vec();
        encoded.extend_from_slice(&ciphertext);
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(encoded))
    }

    fn decode_cursor(&self, cursor: &str) -> Result<CursorClaims, ReplayError> {
        if cursor.len() > MAX_CURSOR_BYTES {
            return Err(ReplayError::ResyncRequired(ResyncReason::InvalidCursor));
        }
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?;
        if encoded.len() <= 24 {
            return Err(ReplayError::ResyncRequired(ResyncReason::InvalidCursor));
        }
        let (nonce, ciphertext) = encoded.split_at(24);
        let plaintext = self
            .cursor_cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?;
        let claims: CursorClaims = serde_json::from_slice(&plaintext)
            .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?;
        if claims.expires_at <= Utc::now().timestamp() {
            return Err(ReplayError::ResyncRequired(ResyncReason::CursorExpired));
        }
        Ok(claims)
    }

    fn validate_cursor_actor(
        &self,
        claims: &CursorClaims,
        actor: &Actor,
        subscriptions: &[ConversationId],
    ) -> Result<(), ReplayError> {
        if claims.protocol_version != PROTOCOL_VERSION {
            return Err(ReplayError::ResyncRequired(ResyncReason::ProtocolChanged));
        }
        if claims.principal_id != actor.user_id().as_str()
            || claims.credential_id != actor.credential_id().as_str()
            || claims.credential_version != actor.credential_version()
        {
            return Err(ReplayError::ResyncRequired(ResyncReason::CredentialChanged));
        }
        if claims.subscription_hash != subscription_hash(subscriptions) {
            return Err(ReplayError::ResyncRequired(
                ResyncReason::SubscriptionChanged,
            ));
        }
        Ok(())
    }
}

async fn resolve_current_event_state(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    recorded_version: i64,
    descriptor: &mut serde_json::Value,
) -> Result<i64, ReplayError> {
    if entity_type == "thread_state" {
        let current: Option<(i64, i64, Option<String>)> = sqlx::query_as(
            "SELECT thread_state_version,archived,thread_archive_reason FROM channels WHERE id=?",
        )
        .bind(entity_id)
        .fetch_optional(&mut *connection)
        .await?;
        if let Some((version, archived, reason)) = current {
            *descriptor = serde_json::json!({
                "archived": archived != 0,
                "reason": reason,
            });
            return Ok(version);
        }
    }
    if entity_type == "thread_tags" {
        let version: Option<i64> =
            sqlx::query_scalar("SELECT thread_tags_version FROM channels WHERE id=?")
                .bind(entity_id)
                .fetch_optional(&mut *connection)
                .await?;
        if let Some(version) = version {
            let tag_ids: Vec<String> = sqlx::query_scalar(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(entity_id)
            .fetch_all(&mut *connection)
            .await?;
            *descriptor = serde_json::json!({
                "thread_id": entity_id,
                "tag_ids": tag_ids,
            });
            return Ok(version);
        }
    }

    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT version FROM entity_versions WHERE entity_type=? AND entity_id=?",
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_optional(&mut *connection)
    .await?
    .unwrap_or(recorded_version))
}

fn canonical_subscriptions(subscriptions: &[String]) -> Result<Vec<ConversationId>, ReplayError> {
    if subscriptions.len() > MAX_SUBSCRIPTIONS {
        return Err(ReplayError::InvalidInput);
    }
    let mut canonical = subscriptions
        .iter()
        .map(|value| ConversationId::from_stored(value.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ReplayError::InvalidInput)?;
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

fn subscription_hash(subscriptions: &[ConversationId]) -> String {
    let mut hasher = Sha256::new();
    for subscription in subscriptions {
        hasher.update((subscription.as_str().len() as u64).to_be_bytes());
        hasher.update(subscription.as_str().as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

async fn authorize_conversation(
    authorization: &AuthorizationService,
    auth: &AuthService,
    connection: &mut SqliteConnection,
    actor: &Actor,
    conversation_id: &str,
) -> Result<(), ReplayError> {
    authorization
        .authorize_conversation_actor_in(
            connection,
            auth,
            actor,
            conversation_id,
            ConversationAction::Read,
        )
        .await
        .map_err(map_authorization_error)
}

fn map_authorization_error(error: AuthorizationError) -> ReplayError {
    match error {
        AuthorizationError::Database(error) => ReplayError::Database(error),
        AuthorizationError::Unavailable => ReplayError::Unavailable,
        AuthorizationError::Authentication(error) => map_auth_error(error),
    }
}

fn map_auth_error(error: crate::auth::authority::AuthError) -> ReplayError {
    match error {
        crate::auth::authority::AuthError::Database(error) => ReplayError::Database(error),
        crate::auth::authority::AuthError::VerificationBusy
        | crate::auth::authority::AuthError::HashWorker(_) => ReplayError::DependencyUnavailable,
        crate::auth::authority::AuthError::Invalid
        | crate::auth::authority::AuthError::Expired
        | crate::auth::authority::AuthError::Revoked
        | crate::auth::authority::AuthError::Disabled
        | crate::auth::authority::AuthError::Token(_) => {
            ReplayError::ResyncRequired(ResyncReason::CredentialChanged)
        }
    }
}

async fn load_snapshot_messages(
    connection: &mut SqliteConnection,
    subscriptions: &[ConversationId],
    message_limit: usize,
) -> Result<Vec<DurableMessageProjection>, ReplayError> {
    if subscriptions.is_empty() {
        return Ok(Vec::new());
    }
    let mut per_conversation = Vec::with_capacity(subscriptions.len());
    for subscription in subscriptions {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM messages WHERE conversation_id=? \
             ORDER BY conversation_sequence DESC LIMIT ?",
        )
        .bind(subscription.as_str())
        .bind(message_limit as i64)
        .fetch_all(&mut *connection)
        .await?;
        per_conversation.push(ids);
    }
    let mut ids = Vec::with_capacity(message_limit);
    for position in 0..message_limit {
        for conversation in &per_conversation {
            if let Some(id) = conversation.get(position) {
                ids.push(id.clone());
                if ids.len() == message_limit {
                    break;
                }
            }
        }
        if ids.len() == message_limit {
            break;
        }
    }
    let mut messages = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(message) = load_message_projection(connection, &id).await? {
            messages.push(message);
        }
    }
    messages.sort_by(|left, right| {
        left.conversation_id
            .cmp(&right.conversation_id)
            .then_with(|| {
                left.sequence
                    .parse::<u64>()
                    .unwrap_or_default()
                    .cmp(&right.sequence.parse::<u64>().unwrap_or_default())
            })
    });
    Ok(messages)
}

async fn load_history_boundaries(
    connection: &mut SqliteConnection,
    messages: &[DurableMessageProjection],
) -> Result<std::collections::BTreeMap<String, String>, ReplayError> {
    let mut earliest = std::collections::BTreeMap::<ConversationId, (&str, i64)>::new();
    for message in messages {
        let sequence = message.sequence.parse::<i64>().unwrap_or_default();
        earliest
            .entry(message.conversation_id.clone())
            .and_modify(|entry| {
                if sequence < entry.1 {
                    *entry = (&message.created_at, sequence);
                }
            })
            .or_insert((&message.created_at, sequence));
    }
    let mut boundaries = std::collections::BTreeMap::new();
    for (conversation_id, (created_at, sequence)) in earliest {
        let older: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages \
             WHERE conversation_id=? AND conversation_sequence<?)",
        )
        .bind(conversation_id.as_str())
        .bind(sequence)
        .fetch_one(&mut *connection)
        .await?;
        if older {
            boundaries.insert(conversation_id.into_inner(), created_at.to_owned());
        }
    }
    Ok(boundaries)
}

async fn load_message_projection(
    connection: &mut SqliteConnection,
    message_id: &str,
) -> Result<Option<DurableMessageProjection>, ReplayError> {
    let row = sqlx::query(
        "SELECT id,conversation_id,conversation_sequence,entity_version,sender_id,sender_nick, \
                content,content_format,created_at,edited_at,deleted_at,reply_to_id, \
                rich_embeds_json,components_json \
         FROM messages WHERE id=? AND conversation_id IS NOT NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let deleted = row.get::<Option<String>, _>(10).is_some();
    let candidate_reply_to_id: Option<String> = (!deleted).then(|| row.get(11)).flatten();
    let reply_to = if let Some(reply_id) = candidate_reply_to_id.as_deref() {
        sqlx::query(
            "SELECT id,sender_id,sender_nick,content,deleted_at FROM messages \
             WHERE id=? AND conversation_id=?",
        )
        .bind(reply_id)
        .bind(row.get::<&str, _>(1))
        .fetch_optional(&mut *connection)
        .await?
        .map(|reply| {
            let reply_deleted = reply.get::<Option<String>, _>(4).is_some();
            DurableReplyProjection {
                message_id: reply.get(0),
                sender_id: reply.get(1),
                sender_nick: reply.get(2),
                content: (!reply_deleted).then(|| reply.get(3)),
                deleted: reply_deleted,
            }
        })
    } else {
        None
    };
    // Historical rows may predate the same-conversation write invariant. Do not
    // expose either their target ID or content through an authorized projection.
    let reply_to_id = reply_to
        .as_ref()
        .map(|reply: &DurableReplyProjection| reply.message_id.clone());
    let attachments = if deleted {
        Vec::new()
    } else {
        sqlx::query(
            "SELECT id,original_filename,content_type,file_size,state_version \
             FROM attachments WHERE message_id=? AND media_state='attached' ORDER BY id",
        )
        .bind(message_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|attachment| DurableAttachmentProjection {
            attachment_id: attachment.get(0),
            filename: attachment.get(1),
            content_type: attachment.get(2),
            file_size: attachment.get(3),
            state_version: attachment.get::<i64, _>(4) as u64,
        })
        .collect()
    };
    let mentions = if deleted {
        Vec::new()
    } else {
        sqlx::query(
            "SELECT mention_kind,target_id,start_byte,end_byte FROM message_mentions \
             WHERE message_id=? ORDER BY ordinal",
        )
        .bind(message_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|mention| {
            let kind = match mention.get::<&str, _>(0) {
                "user" => crate::engine::messaging::MentionKind::User,
                "role" => crate::engine::messaging::MentionKind::Role,
                _ => crate::engine::messaging::MentionKind::Everyone,
            };
            crate::engine::messaging::MessageMention {
                kind,
                target_id: mention.get(1),
                start_byte: mention.get::<i64, _>(2) as usize,
                end_byte: mention.get::<i64, _>(3) as usize,
            }
        })
        .collect()
    };
    let rich_embeds = if deleted {
        None
    } else {
        row.get::<Option<&str>, _>(12)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| ReplayError::DependencyUnavailable)?
    };
    let components = if deleted {
        None
    } else {
        row.get::<Option<&str>, _>(13)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| ReplayError::DependencyUnavailable)?
    };
    Ok(Some(DurableMessageProjection {
        message_id: row.get(0),
        conversation_id: ConversationId::from_stored(row.get::<String, _>(1))
            .map_err(|_| ReplayError::InvalidInput)?,
        sequence: row.get::<i64, _>(2).to_string(),
        entity_version: row.get::<i64, _>(3) as u64,
        sender_id: row.get(4),
        sender_nick: row.get(5),
        content: (!deleted).then(|| row.get(6)),
        content_format: row.get(7),
        created_at: row.get(8),
        edited_at: row.get(9),
        deleted,
        reply_to_id,
        reply_to,
        attachments,
        mentions,
        rich_embeds,
        components,
    }))
}

async fn load_snapshot_reactions(
    connection: &mut SqliteConnection,
    messages: &[DurableMessageProjection],
    principal_id: &str,
) -> Result<Vec<SnapshotReactionGroup>, ReplayError> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder =
        sqlx::QueryBuilder::new("SELECT r.message_id,r.emoji,COUNT(*),MAX(CASE WHEN r.user_id=");
    builder.push_bind(principal_id);
    builder.push(
        " THEN 1 ELSE 0 END) \
         FROM reactions r JOIN messages m ON m.id=r.message_id \
         WHERE m.deleted_at IS NULL AND r.message_id IN (",
    );
    let mut separated = builder.separated(",");
    for message in messages {
        separated.push_bind(&message.message_id);
    }
    separated
        .push_unseparated(") GROUP BY r.message_id,r.emoji ORDER BY r.message_id,r.emoji LIMIT ");
    builder.push_bind((MAX_SNAPSHOT_REACTION_GROUPS + 1) as i64);
    let rows = builder.build().fetch_all(&mut *connection).await?;
    if rows.len() > MAX_SNAPSHOT_REACTION_GROUPS {
        return Err(ReplayError::SnapshotTooLarge);
    }
    let mut reactions = Vec::with_capacity(rows.len());
    for row in rows {
        reactions.push(SnapshotReactionGroup {
            message_id: row.get(0),
            emoji: row.get(1),
            count: row.get::<i64, _>(2) as u64,
            reacted_by_me: row.get::<i64, _>(3) != 0,
        });
    }
    Ok(reactions)
}

async fn load_snapshot_reads(
    connection: &mut SqliteConnection,
    principal_id: &str,
    subscriptions: &[ConversationId],
) -> Result<Vec<DurableReadProjection>, ReplayError> {
    if subscriptions.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT m.conversation_id,rs.last_read_message_id,rs.conversation_sequence \
         FROM read_states rs JOIN messages m ON m.id=rs.last_read_message_id \
         WHERE rs.user_id=",
    );
    builder.push_bind(principal_id);
    builder.push(" AND m.conversation_id IN (");
    let mut separated = builder.separated(",");
    for subscription in subscriptions {
        separated.push_bind(subscription.as_str());
    }
    separated.push_unseparated(") ORDER BY m.conversation_id");
    let rows = builder.build().fetch_all(&mut *connection).await?;
    let mut reads = Vec::with_capacity(rows.len());
    for row in rows {
        let conversation_id = ConversationId::from_stored(row.get::<String, _>(0))
            .map_err(|_| ReplayError::InvalidInput)?;
        let entity_id =
            crate::engine::messaging::read_entity_id(principal_id, conversation_id.as_str());
        let version = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM entity_versions WHERE entity_type='read_state' AND entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&mut *connection)
        .await?
        .unwrap_or(1);
        reads.push(DurableReadProjection {
            conversation_id,
            message_id: row.get(1),
            sequence: row.get::<i64, _>(2).to_string(),
            entity_version: version as u64,
        });
    }
    Ok(reads)
}

async fn load_reaction_projection(
    connection: &mut SqliteConnection,
    entity_id: &str,
    descriptor: &serde_json::Value,
) -> Result<Option<DurableReactionProjection>, ReplayError> {
    let field = |name| {
        descriptor
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or(ReplayError::InvalidInput)
    };
    let message_id = field("message_id")?;
    let user_id = field("user_id")?;
    let emoji = field("emoji")?;
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM reactions r JOIN messages m ON m.id=r.message_id \
         WHERE r.message_id=? AND r.user_id=? AND r.emoji=? AND m.deleted_at IS NULL)",
    )
    .bind(message_id)
    .bind(user_id)
    .bind(emoji)
    .fetch_one(&mut *connection)
    .await?;
    let version = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM entity_versions WHERE entity_type='reaction' AND entity_id=?",
    )
    .bind(entity_id)
    .fetch_optional(&mut *connection)
    .await?
    .unwrap_or(1);
    Ok(Some(DurableReactionProjection {
        message_id: message_id.to_owned(),
        user_id: user_id.to_owned(),
        emoji: emoji.to_owned(),
        present,
        entity_version: version as u64,
    }))
}

async fn load_read_projection(
    connection: &mut SqliteConnection,
    principal_id: &str,
    conversation_id: &str,
    entity_id: &str,
) -> Result<Option<DurableReadProjection>, ReplayError> {
    let row = sqlx::query(
        "SELECT rs.last_read_message_id,rs.conversation_sequence,COALESCE(ev.version,1) \
         FROM read_states rs JOIN messages m ON m.id=rs.last_read_message_id \
         LEFT JOIN entity_versions ev ON ev.entity_type='read_state' AND ev.entity_id=? \
         WHERE rs.user_id=? AND m.conversation_id=?",
    )
    .bind(entity_id)
    .bind(principal_id)
    .bind(conversation_id)
    .fetch_optional(connection)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let conversation_id =
        ConversationId::from_stored(conversation_id).map_err(|_| ReplayError::InvalidInput)?;
    Ok(Some(DurableReadProjection {
        conversation_id,
        message_id: row.get(0),
        sequence: row.get::<i64, _>(1).to_string(),
        entity_version: row.get::<i64, _>(2) as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::engine::messaging::{
        ContentFormat, EntityCommand, MessagingService, ReactionCommand, ReadCommand,
        SendMessageCommand,
    };
    use crate::engine::permissions::DEFAULT_EVERYONE;

    async fn fixture() -> (
        SqlitePool,
        AuthService,
        Actor,
        String,
        MessagingService,
        ReplayService,
    ) {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        for (id, name) in [("user", "carmilla"), ("other", "laura")] {
            sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
                .bind(id)
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) \
             VALUES('server','user','owner'),('server','other','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO roles(id,server_id,name,permissions,is_default) \
             VALUES('everyone','server','@everyone',?,1)",
        )
        .bind(DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
        let actor = auth.issue_web_session("user").await.unwrap().1;
        let messaging = MessagingService::new(pool.clone(), auth.clone(), 4000);
        let replay = ReplayService::new(pool.clone(), auth.clone(), "persistent-secret");
        (pool, auth, actor, conversation, messaging, replay)
    }

    async fn send(
        messaging: &MessagingService,
        actor: &Actor,
        client_id: &str,
        content: &str,
    ) -> crate::engine::messaging::CommandReceipt {
        messaging
            .send_channel_message(
                actor,
                SendMessageCommand {
                    request_id: client_id,
                    client_message_id: client_id,
                    operation_generation: None,
                    conversation_id: None,
                    server_id: "server",
                    channel: "#general",
                    content,
                    content_format: ContentFormat::Markdown,
                    reply_to_id: None,
                    attachment_ids: &[],
                    mentions: &[],
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn cursor_is_opaque_tamper_evident_and_survives_service_restart() {
        let (pool, auth, actor, conversation, messaging, replay) = fixture().await;
        send(&messaging, &actor, "send-1", "first").await;
        let snapshot = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap();
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&snapshot.cursor)
            .unwrap();
        assert!(!String::from_utf8_lossy(&decoded).contains("event_sequence"));
        assert!(!String::from_utf8_lossy(&decoded).contains("user"));

        let restarted = ReplayService::new(pool, auth, "persistent-secret");
        let batch = restarted
            .replay(
                &actor,
                std::slice::from_ref(&conversation),
                &snapshot.cursor,
                100,
            )
            .await
            .unwrap();
        assert!(batch.events.is_empty());

        let mut tampered = snapshot.cursor.into_bytes();
        let last = tampered.last_mut().unwrap();
        *last = if *last == b'A' { b'B' } else { b'A' };
        assert!(matches!(
            restarted
                .replay(
                    &actor,
                    &[conversation],
                    std::str::from_utf8(&tampered).unwrap(),
                    100
                )
                .await,
            Err(ReplayError::ResyncRequired(ResyncReason::InvalidCursor))
        ));
    }

    #[tokio::test]
    async fn replay_preserves_supported_historical_conversation_ids() {
        let (pool, _, actor, conversation, messaging, replay) = fixture().await;
        let historical = format!(" historical conversation:{} ", "界".repeat(300));
        sqlx::query("UPDATE conversations SET id=? WHERE id=?")
            .bind(&historical)
            .bind(&conversation)
            .execute(&pool)
            .await
            .unwrap();

        let initial = replay
            .snapshot(&actor, std::slice::from_ref(&historical))
            .await
            .unwrap();
        send(
            &messaging,
            &actor,
            "historical-conversation-send",
            "preserved",
        )
        .await;
        let batch = replay
            .replay(
                &actor,
                std::slice::from_ref(&historical),
                &initial.cursor,
                100,
            )
            .await
            .unwrap();
        let event = batch
            .events
            .iter()
            .find(|event| event.entity_type == "message")
            .unwrap();
        assert_eq!(event.conversation_id.as_str(), historical);
        assert_eq!(
            event.message.as_ref().unwrap().conversation_id.as_str(),
            historical
        );
        let wire = serde_json::to_value(event).unwrap();
        assert_eq!(wire["conversation_id"], historical);
        assert_eq!(wire["message"]["conversation_id"], historical);
    }

    #[tokio::test]
    async fn cursor_is_bound_to_actor_subscription_and_database_generation() {
        let (pool, auth, actor, conversation, _, replay) = fixture().await;
        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        let other = auth.issue_web_session("other").await.unwrap().1;
        assert!(matches!(
            replay
                .replay(&other, std::slice::from_ref(&conversation), &cursor, 100)
                .await,
            Err(ReplayError::ResyncRequired(ResyncReason::CredentialChanged))
        ));
        assert!(matches!(
            replay
                .replay(&actor, &["different".into()], &cursor, 100)
                .await,
            Err(ReplayError::ResyncRequired(
                ResyncReason::SubscriptionChanged
            ))
        ));
        sqlx::query("UPDATE database_metadata SET generation='restored' WHERE singleton=1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            replay.replay(&actor, &[conversation], &cursor, 100).await,
            Err(ReplayError::ResyncRequired(ResyncReason::DatabaseRestored))
        ));
    }

    #[tokio::test]
    async fn snapshot_then_replay_is_gap_free_and_projects_current_tombstone() {
        let (_, _, actor, conversation, messaging, replay) = fixture().await;
        let first = send(&messaging, &actor, "send-1", "first").await;
        let snapshot = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap();
        assert_eq!(snapshot.messages.len(), 1);
        let second = send(&messaging, &actor, "send-2", "second").await;
        let created = replay
            .replay(
                &actor,
                std::slice::from_ref(&conversation),
                &snapshot.cursor,
                100,
            )
            .await
            .unwrap();
        assert_eq!(created.events.len(), 1);
        assert_eq!(created.events[0].entity_id, second.message_id);

        messaging
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: "delete-1",
                    client_message_id: "delete-1",
                    operation_generation: None,
                    message_id: &first.message_id,
                },
            )
            .await
            .unwrap();
        let fresh = replay.snapshot(&actor, &[conversation]).await.unwrap();
        let tombstone = fresh
            .messages
            .iter()
            .find(|message| message.message_id == first.message_id)
            .unwrap();
        assert!(tombstone.deleted);
        assert!(tombstone.content.is_none());
    }

    #[tokio::test]
    async fn legacy_reply_projection_never_crosses_conversations_and_redacts_deleted_targets() {
        let (pool, _, actor, conversation, _, replay) = fixture().await;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('private','server','#private')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let private_conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='private'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
                                  conversation_id,conversation_sequence,content_format) \
             VALUES('private-target','server','private','other','laura','private text',?,1,'plain'), \
                   ('public-reply','server','channel','user','carmilla','public reply',?,1,'plain')",
        )
        .bind(&private_conversation)
        .bind(&conversation)
        .execute(&pool)
        .await
        .unwrap();
        // Simulate a historical row created before same-conversation reply validation.
        sqlx::query("UPDATE messages SET reply_to_id='private-target' WHERE id='public-reply'")
            .execute(&pool)
            .await
            .unwrap();

        let snapshot = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap();
        let public_reply = snapshot
            .messages
            .iter()
            .find(|message| message.message_id == "public-reply")
            .unwrap();
        assert!(public_reply.reply_to_id.is_none());
        assert!(public_reply.reply_to.is_none());

        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,deleted_at, \
                                  conversation_id,conversation_sequence,content_format) \
             VALUES('deleted-target','server','channel','other','laura','deleted secret',datetime('now'),?,2,'plain'), \
                   ('deleted-reply','server','channel','user','carmilla','same conversation',NULL,?,3,'plain')",
        )
        .bind(&conversation)
        .bind(&conversation)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE messages SET reply_to_id='deleted-target' WHERE id='deleted-reply'")
            .execute(&pool)
            .await
            .unwrap();
        let snapshot = replay.snapshot(&actor, &[conversation]).await.unwrap();
        let deleted_reply = snapshot
            .messages
            .iter()
            .find(|message| message.message_id == "deleted-reply")
            .unwrap()
            .reply_to
            .as_ref()
            .unwrap();
        assert!(deleted_reply.deleted);
        assert!(deleted_reply.content.is_none());
    }

    #[tokio::test]
    async fn replay_projects_a_coherent_current_thread_state() {
        let (pool, _, actor, _, _, replay) = fixture().await;
        sqlx::query(
            "INSERT INTO channels( \
                id,server_id,name,channel_type,parent_channel_id,archived,thread_state_version \
             ) VALUES('thread','server','Thread','public_thread','channel',0,1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='thread'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let mut sequences = Vec::new();
        for (version, archived, reason) in [(2_i64, true, Some("manual")), (3_i64, false, None)] {
            sqlx::query(
                "UPDATE channels SET archived=?,thread_state_version=?,thread_archive_reason=? \
                 WHERE id='thread'",
            )
            .bind(archived)
            .bind(version)
            .bind(reason)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO entity_versions(entity_type,entity_id,version) \
                 VALUES('thread_state','thread',?) \
                 ON CONFLICT(entity_type,entity_id) DO UPDATE SET version=excluded.version",
            )
            .bind(version)
            .execute(&pool)
            .await
            .unwrap();
            let sequence: i64 = sqlx::query_scalar(
                "INSERT INTO event_log( \
                    database_generation,conversation_id,event_kind,entity_type,entity_id, \
                    entity_version,authorization_version,actor_id,descriptor_json \
                 ) VALUES(?,?,'thread_state_changed','thread_state','thread',?,1,'user',?) \
                 RETURNING event_sequence",
            )
            .bind(&generation)
            .bind(&conversation)
            .bind(version)
            .bind(serde_json::json!({"archived": archived, "reason": reason}).to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
            sequences.push(sequence);
        }

        let batch = replay
            .replay(&actor, std::slice::from_ref(&conversation), &cursor, 100)
            .await
            .unwrap();
        assert_eq!(batch.events.len(), 2);
        assert!(batch.events.iter().all(|event| event.entity_version == 3));
        assert!(batch.events.iter().all(|event| {
            event.descriptor == serde_json::json!({"archived": false, "reason": null})
        }));

        let (_, projected) = replay
            .project_event(&actor, sequences[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(projected.entity_version, 3);
        assert_eq!(
            projected.descriptor,
            serde_json::json!({"archived": false, "reason": null})
        );
    }

    #[tokio::test]
    async fn delayed_thread_tag_event_projects_current_tags_and_tag_version_together() {
        let (pool, _, actor, _, _, replay) = fixture().await;
        sqlx::query("UPDATE channels SET channel_type='forum' WHERE id='channel'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels( \
                id,server_id,name,channel_type,parent_channel_id,thread_tags_version \
             ) VALUES('thread','server','Thread','public_thread','channel',1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO forum_tags(id,channel_id,name,position) VALUES \
             ('tag-1','channel','One',0),('tag-2','channel','Two',1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='thread'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let mut sequences = Vec::new();
        for (version, tag_id) in [(2_i64, "tag-1"), (3_i64, "tag-2")] {
            sqlx::query("DELETE FROM thread_tags WHERE thread_id='thread'")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO thread_tags(thread_id,tag_id) VALUES('thread',?)")
                .bind(tag_id)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE channels SET thread_tags_version=? WHERE id='thread'")
                .bind(version)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO entity_versions(entity_type,entity_id,version) \
                 VALUES('thread_tags','thread',?) \
                 ON CONFLICT(entity_type,entity_id) DO UPDATE SET version=excluded.version",
            )
            .bind(version)
            .execute(&pool)
            .await
            .unwrap();
            let sequence: i64 = sqlx::query_scalar(
                "INSERT INTO event_log( \
                    database_generation,conversation_id,event_kind,entity_type,entity_id, \
                    entity_version,authorization_version,actor_id,descriptor_json \
                 ) VALUES(?,?,'thread_tags_updated','thread_tags','thread',?,1,'user',?) \
                 RETURNING event_sequence",
            )
            .bind(&generation)
            .bind(&conversation)
            .bind(version)
            .bind(serde_json::json!({"thread_id":"thread","tag_ids":[tag_id]}).to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
            sequences.push(sequence);
        }

        let batch = replay
            .replay(&actor, std::slice::from_ref(&conversation), &cursor, 100)
            .await
            .unwrap();
        assert_eq!(batch.events.len(), 2);
        assert!(batch.events.iter().all(|event| event.entity_version == 3));
        assert!(batch.events.iter().all(|event| {
            event.descriptor == serde_json::json!({"thread_id":"thread","tag_ids":["tag-2"]})
        }));
        let (_, projected) = replay
            .project_event(&actor, sequences[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(projected.entity_version, 3);
        assert_eq!(
            projected.descriptor,
            serde_json::json!({"thread_id":"thread","tag_ids":["tag-2"]})
        );
    }

    #[tokio::test]
    async fn replay_projects_current_reaction_absence_after_remove_and_parent_delete() {
        let (_, _, actor, conversation, messaging, replay) = fixture().await;
        let sent = send(&messaging, &actor, "send", "message").await;
        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        messaging
            .change_reaction(
                &actor,
                ReactionCommand {
                    request_id: "add",
                    client_message_id: "add",
                    operation_generation: None,
                    message_id: &sent.message_id,
                    emoji: "heart",
                },
                true,
            )
            .await
            .unwrap();
        messaging
            .change_reaction(
                &actor,
                ReactionCommand {
                    request_id: "remove",
                    client_message_id: "remove",
                    operation_generation: None,
                    message_id: &sent.message_id,
                    emoji: "heart",
                },
                false,
            )
            .await
            .unwrap();
        let batch = replay
            .replay(&actor, std::slice::from_ref(&conversation), &cursor, 100)
            .await
            .unwrap();
        let reactions: Vec<_> = batch
            .events
            .iter()
            .filter_map(|event| event.reaction.as_ref())
            .collect();
        assert_eq!(reactions.len(), 2);
        assert!(reactions.iter().all(|reaction| !reaction.present));
        assert!(
            reactions
                .iter()
                .all(|reaction| reaction.entity_version == 2)
        );

        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        messaging
            .change_reaction(
                &actor,
                ReactionCommand {
                    request_id: "add-again",
                    client_message_id: "add-again",
                    operation_generation: None,
                    message_id: &sent.message_id,
                    emoji: "heart",
                },
                true,
            )
            .await
            .unwrap();
        messaging
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: "delete",
                    client_message_id: "delete",
                    operation_generation: None,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
        let batch = replay
            .replay(&actor, &[conversation], &cursor, 100)
            .await
            .unwrap();
        assert!(
            batch
                .events
                .iter()
                .filter_map(|event| event.reaction.as_ref())
                .all(|reaction| !reaction.present)
        );
    }

    #[tokio::test]
    async fn read_state_replay_is_private_to_owning_principal() {
        let (_, auth, actor, conversation, messaging, replay) = fixture().await;
        let sent = send(&messaging, &actor, "send", "message").await;
        let other = auth.issue_web_session("other").await.unwrap().1;
        let other_cursor = replay
            .snapshot(&other, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        messaging
            .mark_read(
                &actor,
                ReadCommand {
                    request_id: "read",
                    client_message_id: "read",
                    operation_generation: None,
                    conversation_id: &conversation,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
        let batch = replay
            .replay(&other, &[conversation], &other_cursor, 100)
            .await
            .unwrap();
        assert!(batch.events.is_empty());
        assert!(!batch.has_more);
    }

    #[tokio::test]
    async fn retention_floor_and_access_revocation_require_explicit_resync() {
        let (pool, _, actor, conversation, _, replay) = fixture().await;
        let empty_cursor = replay.snapshot(&actor, &[]).await.unwrap().cursor;
        sqlx::query("UPDATE event_retention_state SET retained_from_sequence=2 WHERE singleton=1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            replay.replay(&actor, &[], &empty_cursor, 100).await,
            Err(ReplayError::ResyncRequired(ResyncReason::CursorExpired))
        ));

        sqlx::query("UPDATE event_retention_state SET retained_from_sequence=0 WHERE singleton=1")
            .execute(&pool)
            .await
            .unwrap();
        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        sqlx::query("DELETE FROM server_members WHERE server_id='server' AND user_id='user'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            replay.replay(&actor, &[conversation], &cursor, 100).await,
            Err(ReplayError::ResyncRequired(ResyncReason::AccessRevoked))
        ));
    }

    #[tokio::test]
    async fn unrelated_events_do_not_create_observable_empty_pages() {
        let (pool, auth, actor, conversation, _messaging, replay) = fixture().await;
        let cursor = replay
            .snapshot(&actor, std::slice::from_ref(&conversation))
            .await
            .unwrap()
            .cursor;
        let _other = auth.issue_web_session("other").await.unwrap().1;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('other-channel','server','#other')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let other_conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='other-channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<1000) \
             INSERT INTO event_log(database_generation,conversation_id,event_kind,entity_type, \
                                   entity_id,entity_version,authorization_version,actor_id,descriptor_json) \
             SELECT ?,?,'unrelated','metadata','noise-' || n,1,0,'other','{}' FROM numbers",
        )
        .bind(generation)
        .bind(other_conversation)
        .execute(&pool)
        .await
        .unwrap();
        let batch = replay
            .replay(&actor, &[conversation], &cursor, 1)
            .await
            .unwrap();
        assert!(batch.events.is_empty());
        assert!(!batch.has_more);
    }

    #[tokio::test]
    async fn snapshot_reactions_are_scoped_to_the_bounded_message_window() {
        let (pool, _, actor, conversation, _, replay) = fixture().await;
        sqlx::query(
            "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<101) \
             INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at, \
                                  conversation_id,conversation_sequence,content_format,entity_version) \
             SELECT printf('history-%03d',n),'server','channel','user','carmilla','history', \
                    datetime('now'),'channel:' || hex(CAST('channel' AS BLOB)),n,'plain',1 FROM numbers",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO reactions(message_id,user_id,emoji) VALUES \
             ('history-001','user','old'),('history-101','user','bat'), \
             ('history-101','other','bat')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let snapshot = replay.snapshot(&actor, &[conversation]).await.unwrap();
        assert_eq!(snapshot.messages.len(), 100);
        assert_eq!(
            snapshot.reactions,
            vec![SnapshotReactionGroup {
                message_id: "history-101".into(),
                emoji: "bat".into(),
                count: 2,
                reacted_by_me: true,
            }]
        );
        assert!(
            snapshot
                .messages
                .iter()
                .all(|message| message.message_id != "history-001")
        );
    }

    #[tokio::test]
    async fn snapshot_byte_budget_supports_smaller_page_and_marks_older_history() {
        let (pool, _, actor, first_conversation, _, replay) = fixture().await;
        let mut conversations = vec![first_conversation];
        for channel_index in 1..10 {
            let channel_id = format!("channel-{channel_index}");
            let channel_name = format!("#channel-{channel_index}");
            sqlx::query("INSERT INTO channels(id,server_id,name) VALUES(?,'server',?)")
                .bind(&channel_id)
                .bind(channel_name)
                .execute(&pool)
                .await
                .unwrap();
            conversations.push(
                sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id=?")
                    .bind(channel_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
            );
        }
        let content = "🦇".repeat(4000);
        for (conversation_index, conversation_id) in conversations.iter().enumerate() {
            let channel_id = if conversation_index == 0 {
                "channel".to_owned()
            } else {
                format!("channel-{conversation_index}")
            };
            for sequence in 1..=10 {
                sqlx::query(
                    "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
                                          conversation_id,conversation_sequence,content_format) \
                     VALUES(?,'server',?,'user','carmilla',?,?,?,'plain')",
                )
                .bind(format!("large-{conversation_index}-{sequence}"))
                .bind(&channel_id)
                .bind(&content)
                .bind(conversation_id)
                .bind(sequence)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
        let snapshot = replay
            .snapshot_with_limit(&actor, &conversations, 100)
            .await
            .unwrap();
        assert!(snapshot.messages.len() < 100);
        assert!(!snapshot.messages.is_empty());
        assert_eq!(snapshot.history_before.len(), 10);
        let wire_bytes = serde_json::to_vec(&crate::engine::events::ChatEvent::SyncSnapshot {
            request_id: "request".into(),
            snapshot,
        })
        .unwrap()
        .len();
        assert!(wire_bytes < crate::engine::user_session::MAX_OUTBOUND_BYTES);
    }
}
