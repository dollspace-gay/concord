use std::fmt;

use base64::Engine;

use chacha20poly1305::aead::{Aead, KeyInit};

use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

use chrono::Utc;

use rand::Rng;

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
            cursor_cipher: XChaCha20Poly1305::new(&Key::from(key_bytes)),
        }
    }
}

#[cfg(test)]
mod tests;

mod cursors;
mod event_projection;
mod event_replay;
mod event_state;
mod message_projection;
mod snapshot;
mod snapshot_queries;
mod state_projection;
mod subscriptions;
use event_state::resolve_current_event_state;
use message_projection::load_message_projection;
use snapshot_queries::load_history_boundaries;
use snapshot_queries::load_snapshot_messages;
use state_projection::load_reaction_projection;
use state_projection::load_read_projection;
use state_projection::load_snapshot_reactions;
use state_projection::load_snapshot_reads;
use subscriptions::authorize_conversation;
use subscriptions::canonical_subscriptions;
use subscriptions::map_auth_error;
use subscriptions::subscription_hash;
