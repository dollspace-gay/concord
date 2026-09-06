use std::fmt;

#[cfg(feature = "storage-fault-injection")]
use std::time::Duration;

use chrono::Utc;

use schemars::JsonSchema;

use serde::{Deserialize, Serialize};

use sha2::{Digest, Sha256};

use sqlx::{Row, Sqlite, SqliteConnection, SqlitePool, Transaction};

use tokio::sync::{OwnedSemaphorePermit, broadcast};

use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService};

use crate::engine::authorization::{
    AuthorizationError, AuthorizationService, ChannelAction, ConversationAction,
};

use crate::engine::validation;

const MAX_ATTACHMENTS: usize = 10;

const MAX_MENTIONS: usize = 100;

const MAX_CLIENT_ID_BYTES: usize = 128;

const MAX_REQUEST_ID_BYTES: usize = 128;

const RATE_WINDOW_SECONDS: i64 = 10;

const RATE_WINDOW_MESSAGES: i64 = 10;

#[derive(Clone)]
pub struct MessagingService {
    auth: AuthService,
    authorization: AuthorizationService,
    max_message_length: usize,
    write_admission: super::write_admission::WriteAdmission,
    wakeups: broadcast::Sender<u64>,
    #[cfg(feature = "storage-fault-injection")]
    fault_barrier: Option<StorageFaultBarrier>,
}

#[cfg(feature = "storage-fault-injection")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFaultBarrierStage {
    BeforeCommit,
    AfterCommit,
}

#[cfg(feature = "storage-fault-injection")]
#[derive(Clone)]
struct StorageFaultBarrier {
    stage: StorageFaultBarrierStage,
    marker: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendMessageCommand<'a> {
    pub request_id: &'a str,
    pub client_message_id: &'a str,
    /// Required for protocol-v2 clients; trusted legacy adapters pass `None`.
    pub operation_generation: Option<&'a str>,
    /// Canonical v2 target. Legacy adapters may omit it and resolve server/channel in-transaction.
    pub conversation_id: Option<&'a str>,
    pub server_id: &'a str,
    pub channel: &'a str,
    pub content: &'a str,
    pub content_format: ContentFormat,
    pub reply_to_id: Option<&'a str>,
    pub attachment_ids: &'a [String],
    pub mentions: &'a [MessageMention],
}

#[derive(Debug, Clone, Serialize)]
pub struct SendDirectMessageCommand<'a> {
    pub request_id: &'a str,
    pub client_message_id: &'a str,
    /// Required for protocol-v2 clients; trusted legacy adapters pass `None`.
    pub operation_generation: Option<&'a str>,
    /// Stable user ID or registered alias. Resolution happens inside the write transaction.
    pub recipient: &'a str,
    pub content: &'a str,
    pub content_format: ContentFormat,
    pub reply_to_id: Option<&'a str>,
    pub attachment_ids: &'a [String],
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    Plain,
    #[default]
    Markdown,
}

impl ContentFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Markdown => "markdown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct MessageMention {
    pub kind: MentionKind,
    pub target_id: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MentionKind {
    User,
    Role,
    Everyone,
}

impl MentionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Role => "role",
            Self::Everyone => "everyone",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CommandReceipt {
    pub request_id: String,
    pub client_message_id: String,
    pub message_id: String,
    /// Decimal JSON string; JavaScript clients must not parse this as a number.
    pub sequence: String,
    pub entity_version: u64,
    pub persisted_at: String,
    #[serde(default)]
    pub replayed: bool,
    #[serde(skip, default)]
    #[schemars(skip)]
    pub(crate) event_sequence_internal: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditMessageCommand<'a> {
    pub request_id: &'a str,
    pub client_message_id: &'a str,
    pub operation_generation: Option<&'a str>,
    pub message_id: &'a str,
    pub content: &'a str,
    pub content_format: ContentFormat,
    pub mentions: &'a [MessageMention],
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityCommand<'a> {
    pub request_id: &'a str,
    pub client_message_id: &'a str,
    pub operation_generation: Option<&'a str>,
    pub message_id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReactionCommand<'a> {
    pub request_id: &'a str,
    pub client_message_id: &'a str,
    pub operation_generation: Option<&'a str>,
    pub message_id: &'a str,
    pub emoji: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadCommand<'a> {
    pub request_id: &'a str,
    pub client_message_id: &'a str,
    pub operation_generation: Option<&'a str>,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishAnnouncementCommand<'a> {
    pub message_id: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AnnouncementPublication {
    pub publication_id: String,
    pub target_message_id: String,
    pub target_channel_id: String,
}

#[derive(Debug, Clone)]
pub struct MessageMutation {
    pub receipt: CommandReceipt,
    pub conversation_id: String,
    pub channel_id: String,
    pub server_id: String,
    pub content: Option<String>,
    pub emoji: Option<String>,
    pub actor_id: String,
}

struct MessageTarget {
    message_id: String,
    conversation_id: String,
    conversation_sequence: i64,
    server_id: String,
    channel_id: String,
    sender_id: String,
    authorization_version: i64,
    direct: bool,
    deleted: bool,
}

#[derive(Debug)]
pub enum MessagingError {
    Unauthenticated,
    Unavailable,
    InvalidInput(String),
    RateLimited,
    AutoModRejected(String),
    Conflict(String),
    IdempotencyConflict,
    OperationGenerationExpired,
    DependencyUnavailable,
    Internal(sqlx::Error),
}

impl MessagingError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "UNAUTHENTICATED",
            Self::Unavailable => "FORBIDDEN",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::RateLimited => "RATE_LIMITED",
            Self::AutoModRejected(_) => "AUTOMOD_REJECTED",
            Self::Conflict(_) => "CONFLICT",
            Self::IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
            Self::OperationGenerationExpired => "OPERATION_GENERATION_EXPIRED",
            Self::DependencyUnavailable => "DEPENDENCY_UNAVAILABLE",
            Self::Internal(_) => "INTERNAL",
        }
    }

    pub fn safe_message(&self) -> &str {
        match self {
            Self::Unauthenticated => "authentication required",
            Self::Unavailable => "resource unavailable",
            Self::InvalidInput(message) | Self::Conflict(message) => message,
            Self::RateLimited => "message rate limit exceeded",
            Self::AutoModRejected(message) => message,
            Self::IdempotencyConflict => "client message ID was reused with different content",
            Self::OperationGenerationExpired => {
                "operation generation expired; synchronize before retrying"
            }
            Self::DependencyUnavailable => "message storage is temporarily unavailable",
            Self::Internal(_) => "message operation failed",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::DependencyUnavailable)
    }
}

impl fmt::Display for MessagingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.safe_message())
    }
}

impl std::error::Error for MessagingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Internal(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for MessagingError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error)
    }
}

impl MessagingService {
    pub fn new(pool: SqlitePool, auth: AuthService, max_message_length: usize) -> Self {
        let write_admission = super::write_admission::WriteAdmission::new(pool.clone());
        Self::new_with_write_admission(pool, auth, max_message_length, write_admission)
    }
    pub(crate) fn new_with_write_admission(
        pool: SqlitePool,
        auth: AuthService,
        max_message_length: usize,
        write_admission: super::write_admission::WriteAdmission,
    ) -> Self {
        let (wakeups, _) = broadcast::channel(256);
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            auth,
            max_message_length,
            write_admission,
            wakeups,
            #[cfg(feature = "storage-fault-injection")]
            fault_barrier: None,
        }
    }
    #[cfg(feature = "storage-fault-injection")]
    pub fn with_storage_fault_barrier(
        mut self,
        stage: StorageFaultBarrierStage,
        marker: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.fault_barrier = Some(StorageFaultBarrier {
            stage,
            marker: marker.into(),
        });
        self
    }
    #[cfg(feature = "storage-fault-injection")]
    async fn wait_storage_fault_barrier(&self, stage: StorageFaultBarrierStage) {
        if let Some(barrier) = &self.fault_barrier
            && barrier.stage == stage
        {
            std::fs::write(&barrier.marker, format!("{stage:?}\n"))
                .expect("storage fault marker must be writable");
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
    pub fn subscribe_wakeups(&self) -> broadcast::Receiver<u64> {
        self.wakeups.subscribe()
    }
}

#[cfg(test)]
mod tests;

mod announcement_propagation;
mod announcements;
mod channel_commands;
mod channel_write;
mod command_validation;
mod deletion;
mod direct_messages;
mod editing;
mod event_outbox;
mod mentions;
mod operation_receipts;
mod policy;
mod reactions;
mod read_state;
mod receipts;
mod reference_validation;
mod tombstones;
mod transactions;
use announcement_propagation::propagate_announcement_delete;
use announcement_propagation::propagate_announcement_edit;
use command_validation::hash_json;
pub(crate) use command_validation::reaction_entity_id;
pub(crate) use command_validation::read_entity_id;
use command_validation::validate_command;
use command_validation::validate_interaction_response_command;
use command_validation::validate_operation_ids;
use event_outbox::EventIdentity;
use event_outbox::advance_entity_version;
use event_outbox::enqueue_outgoing_webhooks;
use event_outbox::insert_event;
use event_outbox::set_entity_version;
use mentions::insert_mentions;
use operation_receipts::insert_receipt;
use operation_receipts::load_receipt;
use operation_receipts::mutation_receipt;
use operation_receipts::operation_generation;
use policy::enforce_automod;
use policy::enforce_rate_and_slow_mode;
use policy::enforce_timeout;
use policy::map_authorization_error;
use policy::normalize_channel_name;
use receipts::database_generation;
use reference_validation::validate_attachments;
use reference_validation::validate_mentions;
use reference_validation::validate_reply;
use tombstones::tombstone_message_in;
pub(crate) use tombstones::tombstone_moderated_message_in;
