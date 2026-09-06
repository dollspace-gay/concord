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

    pub async fn send_channel_message(
        &self,
        actor: &Actor,
        command: SendMessageCommand<'_>,
    ) -> Result<CommandReceipt, MessagingError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::MessageCommit);
        validate_command(&command, self.max_message_length)?;
        let (permit, mut transaction) = self.begin_write().await?;
        let result = self
            .send_channel_message_in(&mut transaction, actor, &command, command.content)
            .await;
        let result = match result {
            Ok(receipt) => {
                #[cfg(feature = "storage-fault-injection")]
                self.wait_storage_fault_barrier(StorageFaultBarrierStage::BeforeCommit)
                    .await;
                transaction
                    .commit()
                    .await
                    .map_err(MessagingError::Internal)?;
                #[cfg(feature = "storage-fault-injection")]
                self.wait_storage_fault_barrier(StorageFaultBarrierStage::AfterCommit)
                    .await;
                Ok(receipt)
            }
            Err(error @ MessagingError::AutoModRejected(_)) => {
                transaction
                    .commit()
                    .await
                    .map_err(MessagingError::Internal)?;
                Err(error)
            }
            Err(error) => Err(error),
        };
        drop(permit);
        if let Ok(receipt) = &result
            && !receipt.replayed
        {
            let _ = self.wakeups.send(receipt.event_sequence_internal);
        }
        if result.is_ok() {
            metric.succeed();
        }
        result
    }

    /// Commit a public interaction response as a canonical message and consume
    /// the interaction in the same write transaction.
    pub async fn respond_to_interaction_public(
        &self,
        actor: &Actor,
        interaction_id: &str,
        command: SendMessageCommand<'_>,
        rich_embeds_json: Option<&str>,
        components_json: Option<&str>,
    ) -> Result<CommandReceipt, MessagingError> {
        validate_interaction_response_command(
            &command,
            self.max_message_length,
            rich_embeds_json.is_some() || components_json.is_some(),
        )?;
        let (permit, mut transaction) = self.begin_write().await?;
        let receipt = match self
            .send_channel_message_in(&mut transaction, actor, &command, command.content)
            .await
        {
            Ok(receipt) => receipt,
            Err(error @ MessagingError::AutoModRejected(_)) => {
                transaction
                    .commit()
                    .await
                    .map_err(MessagingError::Internal)?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let response_channel: Option<String> =
            sqlx::query_scalar("SELECT channel_id FROM messages WHERE id=?")
                .bind(&receipt.message_id)
                .fetch_optional(&mut *transaction)
                .await?;
        let expected_channel: Option<String> =
            sqlx::query_scalar("SELECT channel_id FROM interactions WHERE id=?")
                .bind(interaction_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if response_channel.is_none() || response_channel != expected_channel {
            return Err(MessagingError::Unavailable);
        }
        sqlx::query("UPDATE messages SET rich_embeds_json=?,components_json=? WHERE id=?")
            .bind(rich_embeds_json)
            .bind(components_json)
            .bind(&receipt.message_id)
            .execute(&mut *transaction)
            .await?;
        use crate::db::queries::slash_commands::InteractionResponseResult;
        match crate::db::queries::slash_commands::accept_interaction_response(
            &mut transaction,
            interaction_id,
            actor.user_id().as_str(),
            Some(&receipt.message_id),
            None,
            None,
        )
        .await?
        {
            InteractionResponseResult::Accepted => {}
            InteractionResponseResult::AlreadyResponded => {
                return Err(MessagingError::Conflict(
                    "interaction already responded".into(),
                ));
            }
            InteractionResponseResult::Expired => {
                return Err(MessagingError::Conflict("interaction expired".into()));
            }
            InteractionResponseResult::WrongApplication | InteractionResponseResult::NotFound => {
                return Err(MessagingError::Unavailable);
            }
        }
        transaction
            .commit()
            .await
            .map_err(MessagingError::Internal)?;
        drop(permit);
        if !receipt.replayed {
            let _ = self.wakeups.send(receipt.event_sequence_internal);
        }
        Ok(receipt)
    }

    /// Explicitly publish one message from a public announcement channel to
    /// every currently authorized follow destination. The lineage uniqueness
    /// constraint makes retries idempotent per follow and source message.
    pub async fn publish_announcement(
        &self,
        actor: &Actor,
        command: PublishAnnouncementCommand<'_>,
    ) -> Result<Vec<AnnouncementPublication>, MessagingError> {
        let (_permit, mut transaction) = self.begin_write().await?;
        let source = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, false)
            .await?;
        if source.direct {
            return Err(MessagingError::Unavailable);
        }
        self.authorization
            .authorize_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &source.channel_id,
                ChannelAction::ManageMessages,
            )
            .await
            .map_err(map_authorization_error)?;
        let source_row = sqlx::query(
            "SELECT m.content,m.content_format,m.entity_version,m.sender_id,m.sender_nick, \
                    c.is_announcement,c.is_private \
             FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.id=?",
        )
        .bind(command.message_id)
        .fetch_one(&mut *transaction)
        .await?;
        if source_row.get::<i64, _>(5) == 0 || source_row.get::<i64, _>(6) != 0 {
            return Err(MessagingError::Unavailable);
        }
        let follows = sqlx::query(
            "SELECT cf.id,cf.target_channel_id,cf.created_by,cv.id,c.server_id,c.authorization_version \
             FROM channel_follows cf \
             JOIN channels c ON c.id=cf.target_channel_id \
             JOIN conversations cv ON cv.channel_id=c.id AND cv.kind='channel' \
             WHERE cf.source_channel_id=? ORDER BY cf.id LIMIT 101",
        )
        .bind(&source.channel_id)
        .fetch_all(&mut *transaction)
        .await?;
        if follows.len() > 100 {
            return Err(MessagingError::Conflict(
                "announcement fanout exceeds 100 destinations".into(),
            ));
        }
        let generation = database_generation(&mut transaction).await?;
        let mut publications = Vec::new();
        let mut wakeup = None;
        for follow in follows {
            let follow_id: String = follow.get(0);
            let target_channel_id: String = follow.get(1);
            let grant_owner: String = follow.get(2);
            if self
                .authorization
                .authorize_channel_in(
                    &mut transaction,
                    &grant_owner,
                    &target_channel_id,
                    ChannelAction::Send,
                )
                .await
                .is_err()
                || self
                    .authorization
                    .authorize_channel_in(
                        &mut transaction,
                        &grant_owner,
                        &target_channel_id,
                        ChannelAction::Manage,
                    )
                    .await
                    .is_err()
            {
                continue;
            }
            if let Some(existing) = sqlx::query(
                "SELECT id,target_message_id FROM announcement_publications \
                 WHERE follow_id=? AND source_message_id=? AND state='published'",
            )
            .bind(&follow_id)
            .bind(command.message_id)
            .fetch_optional(&mut *transaction)
            .await?
            {
                if let Some(target_message_id) = existing.get::<Option<String>, _>(1) {
                    publications.push(AnnouncementPublication {
                        publication_id: existing.get(0),
                        target_message_id,
                        target_channel_id,
                    });
                }
                continue;
            }
            let target_conversation_id: String = follow.get(3);
            let target_server_id: String = follow.get(4);
            let target_authorization_version: i64 = follow.get(5);
            let target_sequence: i64 = sqlx::query_scalar(
                "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
                 WHERE id=? RETURNING next_message_sequence",
            )
            .bind(&target_conversation_id)
            .fetch_one(&mut *transaction)
            .await?;
            let target_message_id = Uuid::new_v4().to_string();
            let publication_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO messages( \
                    id,server_id,channel_id,sender_id,sender_nick,content,conversation_id, \
                    conversation_sequence,content_format,entity_version \
                 ) VALUES(?,?,?,?,?,?,?,?,?,1)",
            )
            .bind(&target_message_id)
            .bind(&target_server_id)
            .bind(&target_channel_id)
            .bind(source_row.get::<&str, _>(3))
            .bind(source_row.get::<&str, _>(4))
            .bind(source_row.get::<&str, _>(0))
            .bind(&target_conversation_id)
            .bind(target_sequence)
            .bind(source_row.get::<&str, _>(1))
            .execute(&mut *transaction)
            .await?;
            set_entity_version(&mut transaction, "message", &target_message_id, 1).await?;
            let target = MessageTarget {
                message_id: target_message_id.clone(),
                conversation_id: target_conversation_id,
                conversation_sequence: target_sequence,
                server_id: target_server_id,
                channel_id: target_channel_id.clone(),
                sender_id: source_row.get(3),
                authorization_version: target_authorization_version,
                direct: false,
                deleted: false,
            };
            let event_sequence = insert_event(
                &mut transaction,
                &generation,
                &target,
                EventIdentity {
                    kind: "message_created",
                    entity_type: "message",
                    entity_id: &target_message_id,
                    version: 1,
                },
                actor.user_id().as_str(),
                serde_json::json!({
                    "conversation_id": target.conversation_id,
                    "message_id": target_message_id,
                    "conversation_sequence": target_sequence.to_string(),
                    "announcement_source_message_id": command.message_id,
                }),
            )
            .await?;
            wakeup = Some(event_sequence as u64);
            sqlx::query(
                "INSERT INTO announcement_publications( \
                    id,follow_id,source_message_id,target_message_id,source_version,state \
                 ) VALUES(?,?,?,?,?,'published')",
            )
            .bind(&publication_id)
            .bind(&follow_id)
            .bind(command.message_id)
            .bind(&target_message_id)
            .bind(source_row.get::<i64, _>(2))
            .execute(&mut *transaction)
            .await?;
            publications.push(AnnouncementPublication {
                publication_id,
                target_message_id,
                target_channel_id,
            });
        }
        if publications.is_empty() {
            return Err(MessagingError::Conflict(
                "announcement has no authorized destinations".into(),
            ));
        }
        transaction.commit().await?;
        if let Some(sequence) = wakeup {
            let _ = self.wakeups.send(sequence);
        }
        Ok(publications)
    }

    pub async fn send_direct_message(
        &self,
        actor: &Actor,
        command: SendDirectMessageCommand<'_>,
    ) -> Result<CommandReceipt, MessagingError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::MessageCommit);
        validate_operation_ids(command.request_id, command.client_message_id)?;
        if command.recipient.is_empty() || command.recipient.len() > 256 {
            return Err(MessagingError::InvalidInput("invalid recipient".into()));
        }
        if command.attachment_ids.len() > MAX_ATTACHMENTS {
            return Err(MessagingError::InvalidInput("too many attachments".into()));
        }
        if command.content.is_empty() && command.attachment_ids.is_empty() {
            return Err(MessagingError::InvalidInput(
                "message content or an attachment is required".into(),
            ));
        }
        if !command.content.is_empty() {
            validation::validate_message_with_limit(command.content, self.max_message_length)
                .map_err(MessagingError::InvalidInput)?;
        }
        let (_permit, mut transaction) = self.begin_write().await?;
        let recipient_id: String = sqlx::query_scalar(
            "SELECT a.user_id FROM user_aliases a JOIN users u ON u.id=a.user_id \
             WHERE a.alias=? COLLATE NOCASE AND u.disabled_at IS NULL LIMIT 1",
        )
        .bind(command.recipient)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        if recipient_id == actor.user_id().as_str() {
            return Err(MessagingError::InvalidInput(
                "cannot send a direct message to yourself".into(),
            ));
        }
        let (lower_user_id, upper_user_id) = if actor.user_id().as_str() < recipient_id.as_str() {
            (actor.user_id().as_str(), recipient_id.as_str())
        } else {
            (recipient_id.as_str(), actor.user_id().as_str())
        };
        let existing_conversation_id: Option<String> = sqlx::query_scalar(
            "SELECT conversation_id FROM direct_conversation_pairs \
             WHERE lower_user_id=? AND upper_user_id=?",
        )
        .bind(lower_user_id)
        .bind(upper_user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let conversation_id = if let Some(existing) = existing_conversation_id {
            existing
        } else {
            let derived: String = sqlx::query_scalar(
                "SELECT 'direct:' || hex(CAST(? AS BLOB)) || ':' || hex(CAST(? AS BLOB))",
            )
            .bind(lower_user_id)
            .bind(upper_user_id)
            .fetch_one(&mut *transaction)
            .await?;
            sqlx::query("INSERT OR IGNORE INTO conversations(id,kind) VALUES(?,'direct')")
                .bind(&derived)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "INSERT OR IGNORE INTO direct_conversation_pairs( \
                     conversation_id,lower_user_id,upper_user_id \
                 ) VALUES(?,?,?)",
            )
            .bind(&derived)
            .bind(lower_user_id)
            .bind(upper_user_id)
            .execute(&mut *transaction)
            .await?;
            let resolved: String = sqlx::query_scalar(
                "SELECT conversation_id FROM direct_conversation_pairs \
                 WHERE lower_user_id=? AND upper_user_id=?",
            )
            .bind(lower_user_id)
            .bind(upper_user_id)
            .fetch_one(&mut *transaction)
            .await?;
            if resolved != derived {
                sqlx::query(
                    "DELETE FROM conversations WHERE id=? AND NOT EXISTS ( \
                         SELECT 1 FROM direct_conversation_pairs WHERE conversation_id=? \
                     )",
                )
                .bind(&derived)
                .bind(&derived)
                .execute(&mut *transaction)
                .await?;
            }
            resolved
        };
        for participant in [lower_user_id, upper_user_id] {
            sqlx::query(
                "INSERT OR IGNORE INTO conversation_participants(conversation_id,user_id) \
                 VALUES(?,?)",
            )
            .bind(&conversation_id)
            .bind(participant)
            .execute(&mut *transaction)
            .await?;
        }
        self.authorization
            .authorize_conversation_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &conversation_id,
                ConversationAction::Send,
            )
            .await
            .map_err(map_authorization_error)?;
        let database_generation = database_generation(&mut transaction).await?;
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "send_direct",
            "conversation_id": conversation_id,
            "recipient_id": recipient_id,
            "content": command.content,
            "content_format": command.content_format,
            "reply_to_id": command.reply_to_id,
            "attachment_ids": command.attachment_ids,
        }))?;
        if let Some(receipt) = load_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            command.client_message_id,
            &fingerprint,
            command.request_id,
        )
        .await?
        {
            transaction.commit().await?;
            metric.succeed();
            return Ok(receipt);
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        let recent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=? AND sender_id=? \
             AND julianday(created_at)>=julianday('now',?)",
        )
        .bind(&conversation_id)
        .bind(actor.user_id().as_str())
        .bind(format!("-{RATE_WINDOW_SECONDS} seconds"))
        .fetch_one(&mut *transaction)
        .await?;
        if recent >= RATE_WINDOW_MESSAGES {
            return Err(MessagingError::RateLimited);
        }
        validate_reply(&mut transaction, &conversation_id, command.reply_to_id).await?;
        validate_attachments(
            &mut transaction,
            actor.user_id().as_str(),
            &conversation_id,
            command.attachment_ids,
        )
        .await?;
        let sequence: i64 = sqlx::query_scalar(
            "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
             WHERE id=? AND kind='direct' RETURNING next_message_sequence",
        )
        .bind(&conversation_id)
        .fetch_one(&mut *transaction)
        .await?;
        let message_id = Uuid::new_v4().to_string();
        let persisted_at = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let sender_nick: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO messages( \
                 id,sender_id,sender_nick,target_user_id,content,created_at,reply_to_id, \
                 conversation_id,conversation_sequence,content_format,entity_version \
             ) VALUES(?,?,?,?,?,?,?,?,?,?,1)",
        )
        .bind(&message_id)
        .bind(actor.user_id().as_str())
        .bind(&sender_nick)
        .bind(&recipient_id)
        .bind(command.content)
        .bind(&persisted_at)
        .bind(command.reply_to_id)
        .bind(&conversation_id)
        .bind(sequence)
        .bind(command.content_format.as_str())
        .execute(&mut *transaction)
        .await?;
        if !command.attachment_ids.is_empty() {
            let mut builder = sqlx::QueryBuilder::new("UPDATE attachments SET message_id=");
            builder.push_bind(&message_id);
            builder
                .push(",media_state='attached',state_version=state_version+1 WHERE uploader_id=");
            builder.push_bind(actor.user_id().as_str());
            builder.push(" AND conversation_id=");
            builder.push_bind(&conversation_id);
            builder.push(
                " AND media_state='ready' AND storage_backend='local' AND storage_key IS NOT NULL \
                 AND message_id IS NULL AND id IN (",
            );
            let mut separated = builder.separated(",");
            for id in command.attachment_ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            if builder
                .build()
                .execute(&mut *transaction)
                .await?
                .rows_affected()
                != command.attachment_ids.len() as u64
            {
                return Err(MessagingError::Conflict(
                    "attachment claim changed during message acceptance".into(),
                ));
            }
        }
        set_entity_version(&mut transaction, "message", &message_id, 1).await?;
        let target = MessageTarget {
            message_id: message_id.clone(),
            conversation_id: conversation_id.clone(),
            conversation_sequence: sequence,
            server_id: String::new(),
            channel_id: String::new(),
            sender_id: actor.user_id().as_str().to_owned(),
            authorization_version: 0,
            direct: true,
            deleted: false,
        };
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: "message_created",
                entity_type: "message",
                entity_id: &message_id,
                version: 1,
            },
            actor.user_id().as_str(),
            serde_json::json!({
                "conversation_id": conversation_id,
                "message_id": message_id,
                "conversation_sequence": sequence.to_string(),
            }),
        )
        .await?;
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            &message_id,
            sequence,
            event_sequence,
            1,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            "send",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        metric.succeed();
        Ok(receipt)
    }

    async fn send_channel_message_in(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
        command: &SendMessageCommand<'_>,
        content: &str,
    ) -> Result<CommandReceipt, MessagingError> {
        let channel = sqlx::query(
            "SELECT c.id,c.server_id,c.archived,c.slowmode_seconds,c.authorization_version,cv.id \
             FROM channels c JOIN conversations cv ON cv.channel_id=c.id AND cv.kind='channel' \
             WHERE ((? IS NOT NULL AND cv.id=?) \
                 OR (? IS NULL AND c.server_id=? AND c.name=?))",
        )
        .bind(command.conversation_id)
        .bind(command.conversation_id)
        .bind(command.conversation_id)
        .bind(command.server_id)
        .bind(normalize_channel_name(command.channel))
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        let channel_id: String = channel.get(0);
        let canonical_server_id: String = channel.get(1);
        let conversation_id: String = channel.get(5);
        if !command.server_id.is_empty() && command.server_id != canonical_server_id {
            return Err(MessagingError::Unavailable);
        }
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "send",
            "conversation_id": conversation_id,
            "content": content,
            "content_format": command.content_format,
            "reply_to_id": command.reply_to_id,
            "attachment_ids": command.attachment_ids,
            "mentions": command.mentions,
        }))?;

        self.authorization
            .authorize_actor_in(
                connection,
                &self.auth,
                actor,
                &channel_id,
                ChannelAction::Send,
            )
            .await
            .map_err(map_authorization_error)?;

        let database_generation = database_generation(connection).await?;
        if let Some(existing) = sqlx::query(
            "SELECT payload_fingerprint,response_json FROM command_receipts \
             WHERE principal_id=? AND client_message_id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(command.client_message_id)
        .fetch_optional(&mut *connection)
        .await?
        {
            if existing.get::<String, _>(0) != fingerprint {
                return Err(MessagingError::IdempotencyConflict);
            }
            let mut receipt: CommandReceipt = serde_json::from_str(existing.get::<&str, _>(1))
                .map_err(|_| MessagingError::DependencyUnavailable)?;
            receipt.request_id = command.request_id.to_owned();
            receipt.replayed = true;
            receipt.event_sequence_internal = sqlx::query_scalar::<_, i64>(
                "SELECT event_sequence FROM command_receipts \
                 WHERE principal_id=? AND client_message_id=?",
            )
            .bind(actor.user_id().as_str())
            .bind(command.client_message_id)
            .fetch_one(&mut *connection)
            .await? as u64;
            return Ok(receipt);
        }
        let operation_generation =
            operation_generation(connection, command.operation_generation).await?;

        if channel.get::<i64, _>(2) != 0 {
            return Err(MessagingError::Conflict(
                "archived channel does not accept messages".into(),
            ));
        }
        enforce_timeout(connection, &canonical_server_id, actor.user_id().as_str()).await?;
        enforce_rate_and_slow_mode(
            connection,
            &channel_id,
            actor.user_id().as_str(),
            channel.get(3),
        )
        .await?;
        enforce_automod(
            connection,
            &canonical_server_id,
            actor.user_id().as_str(),
            command.client_message_id,
            content,
        )
        .await?;
        validate_reply(connection, &conversation_id, command.reply_to_id).await?;
        validate_attachments(
            connection,
            actor.user_id().as_str(),
            &conversation_id,
            command.attachment_ids,
        )
        .await?;
        validate_mentions(connection, &canonical_server_id, content, command.mentions).await?;

        let sequence: i64 = sqlx::query_scalar(
            "UPDATE conversations SET next_message_sequence=next_message_sequence+1 \
             WHERE id=? RETURNING next_message_sequence",
        )
        .bind(&conversation_id)
        .fetch_one(&mut *connection)
        .await?;
        let message_id = Uuid::new_v4().to_string();
        let persisted_at = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let sender_nick: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&mut *connection)
            .await?;
        sqlx::query(
            "INSERT INTO messages( \
                 id,server_id,channel_id,sender_id,sender_nick,content,created_at,reply_to_id, \
                 conversation_id,conversation_sequence,content_format,entity_version \
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,1)",
        )
        .bind(&message_id)
        .bind(&canonical_server_id)
        .bind(&channel_id)
        .bind(actor.user_id().as_str())
        .bind(&sender_nick)
        .bind(content)
        .bind(&persisted_at)
        .bind(command.reply_to_id)
        .bind(&conversation_id)
        .bind(sequence)
        .bind(command.content_format.as_str())
        .execute(&mut *connection)
        .await?;
        crate::db::queries::threads::record_thread_activity(connection, &channel_id, &persisted_at)
            .await?;

        if !command.attachment_ids.is_empty() {
            let mut builder = sqlx::QueryBuilder::new("UPDATE attachments SET message_id=");
            builder.push_bind(&message_id);
            builder.push(",media_state='attached',state_version=state_version+1");
            builder.push(" WHERE uploader_id=");
            builder.push_bind(actor.user_id().as_str());
            builder.push(" AND conversation_id=");
            builder.push_bind(&conversation_id);
            builder.push(
                " AND media_state='ready' AND storage_backend='local' \
                           AND storage_key IS NOT NULL AND message_id IS NULL AND id IN (",
            );
            let mut separated = builder.separated(",");
            for id in command.attachment_ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            let changed = builder.build().execute(&mut *connection).await?;
            if changed.rows_affected() != command.attachment_ids.len() as u64 {
                return Err(MessagingError::Conflict(
                    "attachment claim changed during message acceptance".into(),
                ));
            }
        }
        for (ordinal, mention) in command.mentions.iter().enumerate() {
            sqlx::query(
                "INSERT INTO message_mentions( \
                    message_id,ordinal,mention_kind,target_id,start_byte,end_byte \
                 ) VALUES(?,?,?,?,?,?)",
            )
            .bind(&message_id)
            .bind(ordinal as i64)
            .bind(mention.kind.as_str())
            .bind(&mention.target_id)
            .bind(mention.start_byte as i64)
            .bind(mention.end_byte as i64)
            .execute(&mut *connection)
            .await?;
        }
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version) VALUES('message',?,1)",
        )
        .bind(&message_id)
        .execute(&mut *connection)
        .await?;

        let descriptor = serde_json::json!({
            "conversation_id": conversation_id,
            "message_id": message_id,
            "conversation_sequence": sequence.to_string(),
        });
        let event_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'message_created','message',?,1,?,?,?) RETURNING event_sequence",
        )
        .bind(&database_generation)
        .bind(&conversation_id)
        .bind(&message_id)
        .bind(channel.get::<i64, _>(4))
        .bind(actor.user_id().as_str())
        .bind(descriptor.to_string())
        .fetch_one(&mut *connection)
        .await?;
        sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
            .bind(event_sequence)
            .execute(&mut *connection)
            .await?;
        enqueue_outgoing_webhooks(
            connection,
            event_sequence,
            &MessageTarget {
                message_id: message_id.clone(),
                conversation_id: conversation_id.clone(),
                conversation_sequence: sequence,
                server_id: canonical_server_id.clone(),
                channel_id: channel_id.clone(),
                sender_id: actor.user_id().as_str().to_owned(),
                authorization_version: channel.get(4),
                direct: false,
                deleted: false,
            },
            &EventIdentity {
                kind: "message_created",
                entity_type: "message",
                entity_id: &message_id,
                version: 1,
            },
            actor.user_id().as_str(),
            &descriptor,
        )
        .await?;

        let receipt = CommandReceipt {
            request_id: command.request_id.to_owned(),
            client_message_id: command.client_message_id.to_owned(),
            message_id: message_id.clone(),
            sequence: sequence.to_string(),
            entity_version: 1,
            persisted_at: persisted_at.clone(),
            replayed: false,
            event_sequence_internal: event_sequence as u64,
        };
        sqlx::query(
            "INSERT INTO command_receipts( \
                principal_id,operation_generation,client_message_id,request_id,operation_kind, \
                payload_fingerprint,conversation_id,canonical_message_id,conversation_sequence, \
                event_sequence,entity_version,persisted_at,response_json \
             ) VALUES(?,?,?,?,'send',?,?,?,?,?,1,?,?)",
        )
        .bind(actor.user_id().as_str())
        .bind(&operation_generation)
        .bind(command.client_message_id)
        .bind(command.request_id)
        .bind(&fingerprint)
        .bind(&conversation_id)
        .bind(&message_id)
        .bind(sequence)
        .bind(event_sequence)
        .bind(&persisted_at)
        .bind(serde_json::to_string(&receipt).expect("receipt serialization is infallible"))
        .execute(&mut *connection)
        .await?;
        Ok(receipt)
    }

    pub async fn edit_message(
        &self,
        actor: &Actor,
        command: EditMessageCommand<'_>,
    ) -> Result<MessageMutation, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        validation::validate_message_with_limit(command.content, self.max_message_length)
            .map_err(MessagingError::InvalidInput)?;
        if command.mentions.len() > MAX_MENTIONS {
            return Err(MessagingError::InvalidInput("too many mentions".into()));
        }
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "edit",
            "message_id": command.message_id,
            "content": command.content,
            "content_format": command.content_format,
            "mentions": command.mentions,
        }))?;
        let (_permit, mut transaction) = self.begin_write().await?;
        let target = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, true)
            .await?;
        let database_generation = database_generation(&mut transaction).await?;
        if let Some(receipt) = load_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            command.client_message_id,
            &fingerprint,
            command.request_id,
        )
        .await?
        {
            transaction.commit().await?;
            return Ok(MessageMutation {
                receipt,
                conversation_id: target.conversation_id,
                channel_id: target.channel_id,
                server_id: target.server_id,
                content: Some(command.content.to_owned()),
                emoji: None,
                actor_id: actor.user_id().as_str().to_owned(),
            });
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        if target.sender_id != actor.user_id().as_str() {
            if target.direct {
                return Err(MessagingError::Unavailable);
            }
            self.authorization
                .authorize_actor_in(
                    &mut transaction,
                    &self.auth,
                    actor,
                    &target.channel_id,
                    ChannelAction::ManageMessages,
                )
                .await
                .map_err(map_authorization_error)?;
        }
        match enforce_automod(
            &mut transaction,
            &target.server_id,
            actor.user_id().as_str(),
            command.client_message_id,
            command.content,
        )
        .await
        {
            Ok(()) => {}
            Err(error @ MessagingError::AutoModRejected(_)) => {
                transaction.commit().await?;
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        validate_mentions(
            &mut transaction,
            &target.server_id,
            command.content,
            command.mentions,
        )
        .await?;
        let version: i64 = sqlx::query_scalar(
            "UPDATE messages SET content=?,content_format=?,edited_at=datetime('now'), \
             entity_version=entity_version+1 WHERE id=? AND deleted_at IS NULL \
             RETURNING entity_version",
        )
        .bind(command.content)
        .bind(command.content_format.as_str())
        .bind(command.message_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        sqlx::query("DELETE FROM message_mentions WHERE message_id=?")
            .bind(command.message_id)
            .execute(&mut *transaction)
            .await?;
        insert_mentions(&mut transaction, command.message_id, command.mentions).await?;
        set_entity_version(&mut transaction, "message", command.message_id, version).await?;
        crate::db::queries::atproto::schedule_source_mutation(
            &mut transaction,
            command.message_id,
            version,
            false,
        )
        .await?;
        let persisted_at = Utc::now().to_rfc3339();
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: "message_edited",
                entity_type: "message",
                entity_id: command.message_id,
                version,
            },
            actor.user_id().as_str(),
            serde_json::json!({"message_id": command.message_id}),
        )
        .await?;
        propagate_announcement_edit(
            &mut transaction,
            &database_generation,
            command.message_id,
            command.content,
            command.content_format.as_str(),
            version,
            actor.user_id().as_str(),
        )
        .await?;
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            command.message_id,
            target.conversation_sequence,
            event_sequence,
            version,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            "edit",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        Ok(MessageMutation {
            receipt,
            conversation_id: target.conversation_id,
            channel_id: target.channel_id,
            server_id: target.server_id,
            content: Some(command.content.to_owned()),
            emoji: None,
            actor_id: actor.user_id().as_str().to_owned(),
        })
    }

    pub async fn delete_message(
        &self,
        actor: &Actor,
        command: EntityCommand<'_>,
    ) -> Result<MessageMutation, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "delete",
            "message_id": command.message_id,
        }))?;
        let (_permit, mut transaction) = self.begin_write().await?;
        let target = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, true)
            .await?;
        let database_generation = database_generation(&mut transaction).await?;
        if let Some(receipt) = load_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            command.client_message_id,
            &fingerprint,
            command.request_id,
        )
        .await?
        {
            transaction.commit().await?;
            return Ok(MessageMutation {
                receipt,
                conversation_id: target.conversation_id,
                channel_id: target.channel_id,
                server_id: target.server_id,
                content: None,
                emoji: None,
                actor_id: actor.user_id().as_str().to_owned(),
            });
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        if target.sender_id != actor.user_id().as_str() {
            if target.direct {
                return Err(MessagingError::Unavailable);
            }
            self.authorization
                .authorize_actor_in(
                    &mut transaction,
                    &self.auth,
                    actor,
                    &target.channel_id,
                    ChannelAction::ManageMessages,
                )
                .await
                .map_err(map_authorization_error)?;
        }
        let (version, event_sequence) = tombstone_message_in(
            &mut transaction,
            &database_generation,
            &target,
            actor.user_id().as_str(),
        )
        .await?;
        let persisted_at = Utc::now().to_rfc3339();
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            command.message_id,
            target.conversation_sequence,
            event_sequence,
            version,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            "delete",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        Ok(MessageMutation {
            receipt,
            conversation_id: target.conversation_id,
            channel_id: target.channel_id,
            server_id: target.server_id,
            content: None,
            emoji: None,
            actor_id: actor.user_id().as_str().to_owned(),
        })
    }

    pub async fn change_reaction(
        &self,
        actor: &Actor,
        command: ReactionCommand<'_>,
        add: bool,
    ) -> Result<MessageMutation, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        if command.emoji.is_empty() || command.emoji.chars().count() > 32 {
            return Err(MessagingError::InvalidInput("invalid reaction".into()));
        }
        let operation = if add {
            "reaction_add"
        } else {
            "reaction_remove"
        };
        let fingerprint = hash_json(&serde_json::json!({
            "operation": operation,
            "message_id": command.message_id,
            "emoji": command.emoji,
        }))?;
        let (_permit, mut transaction) = self.begin_write().await?;
        let target = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, true)
            .await?;
        let database_generation = database_generation(&mut transaction).await?;
        if let Some(receipt) = load_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            command.client_message_id,
            &fingerprint,
            command.request_id,
        )
        .await?
        {
            transaction.commit().await?;
            return Ok(MessageMutation {
                receipt,
                conversation_id: target.conversation_id,
                channel_id: target.channel_id,
                server_id: target.server_id,
                content: None,
                emoji: Some(command.emoji.to_owned()),
                actor_id: actor.user_id().as_str().to_owned(),
            });
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        if add {
            sqlx::query("INSERT OR IGNORE INTO reactions(message_id,user_id,emoji) VALUES(?,?,?)")
                .bind(command.message_id)
                .bind(actor.user_id().as_str())
                .bind(command.emoji)
                .execute(&mut *transaction)
                .await?;
        } else {
            sqlx::query("DELETE FROM reactions WHERE message_id=? AND user_id=? AND emoji=?")
                .bind(command.message_id)
                .bind(actor.user_id().as_str())
                .bind(command.emoji)
                .execute(&mut *transaction)
                .await?;
        }
        let reaction_entity =
            reaction_entity_id(command.message_id, actor.user_id().as_str(), command.emoji);
        let version =
            advance_entity_version(&mut transaction, "reaction", &reaction_entity).await?;
        let persisted_at = Utc::now().to_rfc3339();
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: if add {
                    "reaction_added"
                } else {
                    "reaction_removed"
                },
                entity_type: "reaction",
                entity_id: &reaction_entity,
                version,
            },
            actor.user_id().as_str(),
            serde_json::json!({
                "message_id": command.message_id,
                "user_id": actor.user_id().as_str(),
                "emoji": command.emoji,
                "present": add,
            }),
        )
        .await?;
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            command.message_id,
            target.conversation_sequence,
            event_sequence,
            version,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            operation,
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        Ok(MessageMutation {
            receipt,
            conversation_id: target.conversation_id,
            channel_id: target.channel_id,
            server_id: target.server_id,
            content: None,
            emoji: Some(command.emoji.to_owned()),
            actor_id: actor.user_id().as_str().to_owned(),
        })
    }

    pub async fn mark_read(
        &self,
        actor: &Actor,
        command: ReadCommand<'_>,
    ) -> Result<CommandReceipt, MessagingError> {
        validate_operation_ids(command.request_id, command.client_message_id)?;
        let fingerprint = hash_json(&serde_json::json!({
            "operation": "read",
            "conversation_id": command.conversation_id,
            "message_id": command.message_id,
        }))?;
        let (_permit, mut transaction) = self.begin_write().await?;
        let target = self
            .load_and_authorize_message(&mut transaction, actor, command.message_id, true)
            .await?;
        if target.conversation_id != command.conversation_id {
            return Err(MessagingError::Unavailable);
        }
        let database_generation = database_generation(&mut transaction).await?;
        if let Some(receipt) = load_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            command.client_message_id,
            &fingerprint,
            command.request_id,
        )
        .await?
        {
            transaction.commit().await?;
            return Ok(receipt);
        }
        let operation_generation =
            operation_generation(&mut transaction, command.operation_generation).await?;
        if target.deleted {
            return Err(MessagingError::Unavailable);
        }
        let read_key = if target.direct {
            &target.conversation_id
        } else {
            &target.channel_id
        };
        let current_read_sequence: Option<i64> = sqlx::query_scalar(
            "SELECT conversation_sequence FROM read_states WHERE user_id=? AND channel_id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(read_key)
        .fetch_optional(&mut *transaction)
        .await?;
        if current_read_sequence.is_some_and(|sequence| sequence >= target.conversation_sequence) {
            let read_entity = read_entity_id(actor.user_id().as_str(), command.conversation_id);
            let (version, event_sequence): (i64, i64) = sqlx::query_as(
                "SELECT \
                    COALESCE((SELECT version FROM entity_versions \
                              WHERE entity_type='read_state' AND entity_id=?),1), \
                    COALESCE((SELECT MAX(event_sequence) FROM event_log \
                              WHERE entity_type='read_state' AND entity_id=?),0)",
            )
            .bind(&read_entity)
            .bind(&read_entity)
            .fetch_one(&mut *transaction)
            .await?;
            let persisted_at = Utc::now().to_rfc3339();
            let receipt = mutation_receipt(
                command.request_id,
                command.client_message_id,
                command.message_id,
                target.conversation_sequence,
                event_sequence,
                version,
                &persisted_at,
            );
            insert_receipt(
                &mut transaction,
                actor.user_id().as_str(),
                &operation_generation,
                "read",
                &fingerprint,
                &target,
                &receipt,
            )
            .await?;
            transaction.commit().await?;
            return Ok(receipt);
        }
        sqlx::query(
            "INSERT INTO read_states( \
                 user_id,channel_id,last_read_message_id,last_read_at,conversation_sequence \
             ) VALUES(?,?,?,datetime('now'),?) \
             ON CONFLICT(user_id,channel_id) DO UPDATE SET \
                 last_read_message_id=CASE \
                     WHEN excluded.conversation_sequence > read_states.conversation_sequence \
                     THEN excluded.last_read_message_id ELSE read_states.last_read_message_id END, \
                 conversation_sequence=MAX(read_states.conversation_sequence,excluded.conversation_sequence), \
                 last_read_at=CASE \
                     WHEN excluded.conversation_sequence > read_states.conversation_sequence \
                     THEN excluded.last_read_at ELSE read_states.last_read_at END",
        )
        .bind(actor.user_id().as_str())
        .bind(read_key)
        .bind(command.message_id)
        .bind(target.conversation_sequence)
        .execute(&mut *transaction)
        .await?;
        let read_entity = read_entity_id(actor.user_id().as_str(), command.conversation_id);
        let version = advance_entity_version(&mut transaction, "read_state", &read_entity).await?;
        let persisted_at = Utc::now().to_rfc3339();
        let event_sequence = insert_event(
            &mut transaction,
            &database_generation,
            &target,
            EventIdentity {
                kind: "read_advanced",
                entity_type: "read_state",
                entity_id: &read_entity,
                version,
            },
            actor.user_id().as_str(),
            serde_json::json!({
                "user_id": actor.user_id().as_str(),
                "conversation_id": command.conversation_id,
                "message_id": command.message_id,
                "conversation_sequence": target.conversation_sequence.to_string(),
            }),
        )
        .await?;
        let receipt = mutation_receipt(
            command.request_id,
            command.client_message_id,
            command.message_id,
            target.conversation_sequence,
            event_sequence,
            version,
            &persisted_at,
        );
        insert_receipt(
            &mut transaction,
            actor.user_id().as_str(),
            &operation_generation,
            "read",
            &fingerprint,
            &target,
            &receipt,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.wakeups.send(event_sequence as u64);
        Ok(receipt)
    }

    async fn begin_write(
        &self,
    ) -> Result<(OwnedSemaphorePermit, Transaction<'static, Sqlite>), MessagingError> {
        self.write_admission
            .begin()
            .await
            .map_err(|error| match error {
                super::write_admission::WriteAdmissionError::Unavailable => {
                    MessagingError::DependencyUnavailable
                }
                super::write_admission::WriteAdmissionError::Database(error) => {
                    MessagingError::Internal(error)
                }
            })
    }

    async fn load_and_authorize_message(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
        message_id: &str,
        allow_deleted: bool,
    ) -> Result<MessageTarget, MessagingError> {
        let row = sqlx::query(
            "SELECT m.id,m.conversation_id,m.conversation_sequence,COALESCE(m.server_id,''), \
                    COALESCE(m.channel_id,''),m.sender_id,COALESCE(c.authorization_version,0), \
                    m.deleted_at,cv.kind \
             FROM messages m JOIN conversations cv ON cv.id=m.conversation_id \
             LEFT JOIN channels c ON c.id=m.channel_id \
             WHERE m.id=? AND m.conversation_id IS NOT NULL",
        )
        .bind(message_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(MessagingError::Unavailable)?;
        if !allow_deleted && row.get::<Option<String>, _>(7).is_some() {
            return Err(MessagingError::Unavailable);
        }
        let target = MessageTarget {
            message_id: row.get(0),
            conversation_id: row.get(1),
            conversation_sequence: row.get(2),
            server_id: row.get(3),
            channel_id: row.get(4),
            sender_id: row.get(5),
            authorization_version: row.get(6),
            direct: row.get::<String, _>(8) == "direct",
            deleted: row.get::<Option<String>, _>(7).is_some(),
        };
        self.authorization
            .authorize_conversation_actor_in(
                connection,
                &self.auth,
                actor,
                &target.conversation_id,
                ConversationAction::Read,
            )
            .await
            .map_err(map_authorization_error)?;
        Ok(target)
    }
}

fn validate_command(
    command: &SendMessageCommand<'_>,
    max_message_length: usize,
) -> Result<(), MessagingError> {
    if command.request_id.is_empty() || command.request_id.len() > MAX_REQUEST_ID_BYTES {
        return Err(MessagingError::InvalidInput("invalid request ID".into()));
    }
    if command.client_message_id.is_empty() || command.client_message_id.len() > MAX_CLIENT_ID_BYTES
    {
        return Err(MessagingError::InvalidInput(
            "invalid client message ID".into(),
        ));
    }
    if command.attachment_ids.len() > MAX_ATTACHMENTS {
        return Err(MessagingError::InvalidInput("too many attachments".into()));
    }
    if command.mentions.len() > MAX_MENTIONS {
        return Err(MessagingError::InvalidInput("too many mentions".into()));
    }
    if command.content.is_empty() && command.attachment_ids.is_empty() {
        return Err(MessagingError::InvalidInput(
            "message content or an attachment is required".into(),
        ));
    }
    if !command.content.is_empty() {
        validation::validate_message_with_limit(command.content, max_message_length)
            .map_err(MessagingError::InvalidInput)?;
    }
    Ok(())
}

fn validate_interaction_response_command(
    command: &SendMessageCommand<'_>,
    max_message_length: usize,
    has_rich_content: bool,
) -> Result<(), MessagingError> {
    if command.content.is_empty() && command.attachment_ids.is_empty() && has_rich_content {
        let mut validation_command = command.clone();
        validation_command.content = "interaction response";
        validate_command(&validation_command, max_message_length)
    } else {
        validate_command(command, max_message_length)
    }
}

fn validate_operation_ids(request_id: &str, client_message_id: &str) -> Result<(), MessagingError> {
    if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_BYTES {
        return Err(MessagingError::InvalidInput("invalid request ID".into()));
    }
    if client_message_id.is_empty() || client_message_id.len() > MAX_CLIENT_ID_BYTES {
        return Err(MessagingError::InvalidInput(
            "invalid client message ID".into(),
        ));
    }
    Ok(())
}

fn hash_json(value: &serde_json::Value) -> Result<String, MessagingError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| MessagingError::InvalidInput("command cannot be encoded".into()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn domain_tuple_id(domain: &[u8], fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn reaction_entity_id(message_id: &str, user_id: &str, emoji: &str) -> String {
    domain_tuple_id(b"concord:reaction:v1", &[message_id, user_id, emoji])
}

pub(crate) fn read_entity_id(user_id: &str, conversation_id: &str) -> String {
    domain_tuple_id(b"concord:read-state:v1", &[user_id, conversation_id])
}

async fn database_generation(connection: &mut SqliteConnection) -> Result<String, MessagingError> {
    Ok(
        sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
            .fetch_one(connection)
            .await?,
    )
}

async fn tombstone_message_in(
    connection: &mut SqliteConnection,
    generation: &str,
    target: &MessageTarget,
    actor_id: &str,
) -> Result<(i64, i64), MessagingError> {
    let version: i64 = sqlx::query_scalar(
        "UPDATE messages SET deleted_at=datetime('now'),entity_version=entity_version+1 \
         WHERE id=? AND deleted_at IS NULL RETURNING entity_version",
    )
    .bind(&target.message_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(MessagingError::Unavailable)?;
    sqlx::query(
        "UPDATE attachments SET media_state='deleting',state_version=state_version+1, \
         delete_after=datetime('now','+1 hour') \
         WHERE message_id=? AND media_state='attached'",
    )
    .bind(&target.message_id)
    .execute(&mut *connection)
    .await?;
    set_entity_version(connection, "message", &target.message_id, version).await?;
    crate::db::queries::atproto::schedule_source_mutation(
        connection,
        &target.message_id,
        version,
        true,
    )
    .await?;
    let event_sequence = insert_event(
        connection,
        generation,
        target,
        EventIdentity {
            kind: "message_deleted",
            entity_type: "message",
            entity_id: &target.message_id,
            version,
        },
        actor_id,
        serde_json::json!({"message_id": target.message_id}),
    )
    .await?;
    propagate_announcement_delete(
        connection,
        generation,
        &target.message_id,
        version,
        actor_id,
    )
    .await?;
    Ok((version, event_sequence))
}

/// Canonical deletion primitive for an already-authorized durable moderation
/// job. Returning `None` lets a resumed batch skip a concurrently completed
/// tombstone without inventing a second version or event.
pub(crate) async fn tombstone_moderated_message_in(
    connection: &mut SqliteConnection,
    generation: &str,
    message_id: &str,
    actor_id: &str,
) -> Result<Option<i64>, MessagingError> {
    let row = sqlx::query(
        "SELECT m.id,m.conversation_id,m.conversation_sequence,m.server_id,m.channel_id, \
                m.sender_id,c.authorization_version,m.deleted_at \
         FROM messages m JOIN channels c ON c.id=m.channel_id AND c.server_id=m.server_id \
         WHERE m.id=? AND m.deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let target = MessageTarget {
        message_id: row.get(0),
        conversation_id: row.get(1),
        conversation_sequence: row.get(2),
        server_id: row.get(3),
        channel_id: row.get(4),
        sender_id: row.get(5),
        authorization_version: row.get(6),
        direct: false,
        deleted: row.get::<Option<String>, _>(7).is_some(),
    };
    let (_, event_sequence) =
        tombstone_message_in(connection, generation, &target, actor_id).await?;
    Ok(Some(event_sequence))
}

async fn operation_generation(
    connection: &mut SqliteConnection,
    requested: Option<&str>,
) -> Result<String, MessagingError> {
    let row = sqlx::query(
        "SELECT s.current_generation,g.expires_at \
         FROM operation_generation_state s \
         JOIN operation_generations g ON g.generation=s.current_generation \
         WHERE s.singleton=1",
    )
    .fetch_one(&mut *connection)
    .await?;
    let current: String = row.get(0);
    let expires_at: i64 = row.get(1);
    let now: i64 = sqlx::query_scalar("SELECT unixepoch()")
        .fetch_one(&mut *connection)
        .await?;
    if expires_at <= now {
        if requested.is_some() {
            return Err(MessagingError::OperationGenerationExpired);
        }
        let next: String = sqlx::query_scalar("SELECT lower(hex(randomblob(16)))")
            .fetch_one(&mut *connection)
            .await?;
        sqlx::query(
            "INSERT INTO operation_generations(generation,issued_at,expires_at) \
             VALUES(?,?,?)",
        )
        .bind(&next)
        .bind(now)
        .bind(now + 604_800)
        .execute(&mut *connection)
        .await?;
        sqlx::query("UPDATE operation_generation_state SET current_generation=? WHERE singleton=1")
            .bind(&next)
            .execute(&mut *connection)
            .await?;
        return Ok(next);
    }
    if requested.is_some_and(|generation| generation != current) {
        return Err(MessagingError::OperationGenerationExpired);
    }
    Ok(current)
}

async fn load_receipt(
    connection: &mut SqliteConnection,
    principal_id: &str,
    client_message_id: &str,
    fingerprint: &str,
    request_id: &str,
) -> Result<Option<CommandReceipt>, MessagingError> {
    let row = sqlx::query(
        "SELECT payload_fingerprint,response_json,event_sequence FROM command_receipts \
         WHERE principal_id=? AND client_message_id=?",
    )
    .bind(principal_id)
    .bind(client_message_id)
    .fetch_optional(connection)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<String, _>(0) != fingerprint {
        return Err(MessagingError::IdempotencyConflict);
    }
    let mut receipt: CommandReceipt = serde_json::from_str(row.get::<&str, _>(1))
        .map_err(|_| MessagingError::DependencyUnavailable)?;
    receipt.request_id = request_id.to_owned();
    receipt.replayed = true;
    receipt.event_sequence_internal = row.get::<i64, _>(2) as u64;
    Ok(Some(receipt))
}

fn mutation_receipt(
    request_id: &str,
    client_message_id: &str,
    message_id: &str,
    conversation_sequence: i64,
    event_sequence: i64,
    entity_version: i64,
    persisted_at: &str,
) -> CommandReceipt {
    CommandReceipt {
        request_id: request_id.to_owned(),
        client_message_id: client_message_id.to_owned(),
        message_id: message_id.to_owned(),
        sequence: conversation_sequence.to_string(),
        entity_version: entity_version as u64,
        persisted_at: persisted_at.to_owned(),
        replayed: false,
        event_sequence_internal: event_sequence as u64,
    }
}

async fn insert_receipt(
    connection: &mut SqliteConnection,
    principal_id: &str,
    generation: &str,
    operation_kind: &str,
    fingerprint: &str,
    target: &MessageTarget,
    receipt: &CommandReceipt,
) -> Result<(), MessagingError> {
    sqlx::query(
        "INSERT INTO command_receipts( \
            principal_id,operation_generation,client_message_id,request_id,operation_kind, \
            payload_fingerprint,conversation_id,canonical_message_id,conversation_sequence, \
            event_sequence,entity_version,persisted_at,response_json \
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(principal_id)
    .bind(generation)
    .bind(&receipt.client_message_id)
    .bind(&receipt.request_id)
    .bind(operation_kind)
    .bind(fingerprint)
    .bind(&target.conversation_id)
    .bind(&target.message_id)
    .bind(target.conversation_sequence)
    .bind(receipt.event_sequence_internal as i64)
    .bind(receipt.entity_version as i64)
    .bind(&receipt.persisted_at)
    .bind(serde_json::to_string(receipt).expect("receipt serialization is infallible"))
    .execute(connection)
    .await?;
    Ok(())
}

async fn propagate_announcement_edit(
    connection: &mut SqliteConnection,
    generation: &str,
    source_message_id: &str,
    content: &str,
    content_format: &str,
    source_version: i64,
    actor_id: &str,
) -> Result<(), MessagingError> {
    let targets = sqlx::query(
        "SELECT ap.id,m.id,m.conversation_id,m.conversation_sequence,m.server_id,m.channel_id, \
                c.authorization_version \
         FROM announcement_publications ap \
         JOIN messages m ON m.id=ap.target_message_id \
         JOIN channels c ON c.id=m.channel_id \
         WHERE ap.source_message_id=? AND ap.state='published' AND m.deleted_at IS NULL",
    )
    .bind(source_message_id)
    .fetch_all(&mut *connection)
    .await?;
    for row in targets {
        let target_message_id: String = row.get(1);
        let target_version: i64 = sqlx::query_scalar(
            "UPDATE messages SET content=?,content_format=?,edited_at=datetime('now'), \
             entity_version=entity_version+1 WHERE id=? RETURNING entity_version",
        )
        .bind(content)
        .bind(content_format)
        .bind(&target_message_id)
        .fetch_one(&mut *connection)
        .await?;
        set_entity_version(connection, "message", &target_message_id, target_version).await?;
        let target = MessageTarget {
            message_id: target_message_id.clone(),
            conversation_id: row.get(2),
            conversation_sequence: row.get(3),
            server_id: row.get(4),
            channel_id: row.get(5),
            sender_id: actor_id.to_string(),
            authorization_version: row.get(6),
            direct: false,
            deleted: false,
        };
        insert_event(
            connection,
            generation,
            &target,
            EventIdentity {
                kind: "message_edited",
                entity_type: "message",
                entity_id: &target_message_id,
                version: target_version,
            },
            actor_id,
            serde_json::json!({
                "message_id": target_message_id,
                "announcement_source_message_id": source_message_id,
            }),
        )
        .await?;
        sqlx::query(
            "UPDATE announcement_publications SET source_version=?,updated_at=datetime('now') \
             WHERE id=?",
        )
        .bind(source_version)
        .bind(row.get::<String, _>(0))
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn propagate_announcement_delete(
    connection: &mut SqliteConnection,
    generation: &str,
    source_message_id: &str,
    source_version: i64,
    actor_id: &str,
) -> Result<(), MessagingError> {
    let targets = sqlx::query(
        "SELECT ap.id,m.id,m.conversation_id,m.conversation_sequence,m.server_id,m.channel_id, \
                c.authorization_version \
         FROM announcement_publications ap \
         JOIN messages m ON m.id=ap.target_message_id \
         JOIN channels c ON c.id=m.channel_id \
         WHERE ap.source_message_id=? AND ap.state='published'",
    )
    .bind(source_message_id)
    .fetch_all(&mut *connection)
    .await?;
    for row in targets {
        let target_message_id: String = row.get(1);
        let target_version: i64 = sqlx::query_scalar(
            "UPDATE messages SET deleted_at=COALESCE(deleted_at,datetime('now')), \
             entity_version=entity_version+1 WHERE id=? RETURNING entity_version",
        )
        .bind(&target_message_id)
        .fetch_one(&mut *connection)
        .await?;
        set_entity_version(connection, "message", &target_message_id, target_version).await?;
        let target = MessageTarget {
            message_id: target_message_id.clone(),
            conversation_id: row.get(2),
            conversation_sequence: row.get(3),
            server_id: row.get(4),
            channel_id: row.get(5),
            sender_id: actor_id.to_string(),
            authorization_version: row.get(6),
            direct: false,
            deleted: true,
        };
        insert_event(
            connection,
            generation,
            &target,
            EventIdentity {
                kind: "message_deleted",
                entity_type: "message",
                entity_id: &target_message_id,
                version: target_version,
            },
            actor_id,
            serde_json::json!({
                "message_id": target_message_id,
                "announcement_source_message_id": source_message_id,
            }),
        )
        .await?;
        sqlx::query(
            "UPDATE announcement_publications SET state='deleted',source_version=?, \
             updated_at=datetime('now') WHERE id=?",
        )
        .bind(source_version)
        .bind(row.get::<String, _>(0))
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

struct EventIdentity<'a> {
    kind: &'a str,
    entity_type: &'a str,
    entity_id: &'a str,
    version: i64,
}

async fn insert_event(
    connection: &mut SqliteConnection,
    generation: &str,
    target: &MessageTarget,
    event: EventIdentity<'_>,
    actor_id: &str,
    descriptor: serde_json::Value,
) -> Result<i64, MessagingError> {
    let event_sequence: i64 = sqlx::query_scalar(
        "INSERT INTO event_log( \
            database_generation,conversation_id,event_kind,entity_type,entity_id, \
            entity_version,authorization_version,actor_id,descriptor_json \
         ) VALUES(?,?,?,?,?,?,?,?,?) RETURNING event_sequence",
    )
    .bind(generation)
    .bind(&target.conversation_id)
    .bind(event.kind)
    .bind(event.entity_type)
    .bind(event.entity_id)
    .bind(event.version)
    .bind(target.authorization_version)
    .bind(actor_id)
    .bind(descriptor.to_string())
    .fetch_one(&mut *connection)
    .await?;
    sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
        .bind(event_sequence)
        .execute(&mut *connection)
        .await?;
    enqueue_outgoing_webhooks(
        connection,
        event_sequence,
        target,
        &event,
        actor_id,
        &descriptor,
    )
    .await?;
    Ok(event_sequence)
}

async fn enqueue_outgoing_webhooks(
    connection: &mut SqliteConnection,
    event_sequence: i64,
    target: &MessageTarget,
    event: &EventIdentity<'_>,
    actor_id: &str,
    descriptor: &serde_json::Value,
) -> Result<(), MessagingError> {
    let event_type = match event.kind {
        "message_created" => "message_create",
        "message_edited" => "message_update",
        "message_deleted" => "message_delete",
        other => other,
    };
    let mut outbound_data = descriptor.clone();
    if matches!(event_type, "message_create" | "message_update") {
        let row: Option<(String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT content,content_format,sender_id,sender_nick,edited_at FROM messages \
             WHERE id=? AND channel_id=? AND deleted_at IS NULL",
        )
        .bind(event.entity_id)
        .bind(&target.channel_id)
        .fetch_optional(&mut *connection)
        .await?;
        if let (Some(fields), Some(data)) = (row, outbound_data.as_object_mut()) {
            data.insert("content".into(), fields.0.into());
            data.insert("content_format".into(), fields.1.into());
            data.insert("sender_id".into(), fields.2.into());
            data.insert("sender_nick".into(), fields.3.into());
            data.insert("edited_at".into(), fields.4.into());
        }
    }
    let webhooks: Vec<(String, i64)> = sqlx::query_as(
        "SELECT w.id,w.grant_version FROM webhooks w \
         JOIN webhook_events e ON e.webhook_id=w.id \
         JOIN channels c ON c.id=w.channel_id AND c.server_id=w.server_id \
         WHERE w.server_id=? AND w.channel_id=? AND c.is_private=0 \
           AND w.webhook_type='outgoing' \
           AND w.credential_state='active' AND w.revoked_at IS NULL AND e.event_type=?",
    )
    .bind(&target.server_id)
    .bind(&target.channel_id)
    .bind(event_type)
    .fetch_all(&mut *connection)
    .await?;
    for (webhook_id, grant_version) in webhooks {
        let job_id = Uuid::new_v4().to_string();
        let delivery_id = Uuid::new_v4().to_string();
        let deduplication_key = format!("webhook:{webhook_id}:event:{event_sequence}");
        let destination_grant = format!("webhook:{webhook_id}:{grant_version}");
        let payload = serde_json::json!({
            "delivery_id": delivery_id,
            "event_type": event_type,
            "event_version": event.version,
            "entity_type": event.entity_type,
            "entity_id": event.entity_id,
            "actor_id": actor_id,
            "server_id": target.server_id,
            "channel_id": target.channel_id,
            "conversation_id": target.conversation_id,
            "data": &outbound_data,
        });
        sqlx::query(
            "INSERT INTO external_jobs( \
                id,deduplication_key,operation_type,resource_id,resource_version, \
                destination_grant,payload_json \
             ) VALUES(?,?,'webhook_delivery',?,?,?,?)",
        )
        .bind(&job_id)
        .bind(&deduplication_key)
        .bind(&delivery_id)
        .bind(event.version)
        .bind(&destination_grant)
        .bind(payload.to_string())
        .execute(&mut *connection)
        .await?;
        sqlx::query(
            "INSERT INTO webhook_deliveries( \
                id,webhook_id,event_sequence,external_job_id,delivery_id,event_type, \
                event_version,payload_json \
             ) VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&webhook_id)
        .bind(event_sequence)
        .bind(&job_id)
        .bind(&delivery_id)
        .bind(event_type)
        .bind(event.version)
        .bind(payload.to_string())
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn set_entity_version(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    version: i64,
) -> Result<(), MessagingError> {
    sqlx::query(
        "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
         VALUES(?,?,?,datetime('now')) \
         ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
             version=excluded.version,updated_at=excluded.updated_at",
    )
    .bind(entity_type)
    .bind(entity_id)
    .bind(version)
    .execute(connection)
    .await?;
    Ok(())
}

async fn advance_entity_version(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> Result<i64, MessagingError> {
    Ok(sqlx::query_scalar(
        "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
         VALUES(?,?,1,datetime('now')) \
         ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
             version=entity_versions.version+1,updated_at=excluded.updated_at \
         RETURNING version",
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_one(connection)
    .await?)
}

async fn insert_mentions(
    connection: &mut SqliteConnection,
    message_id: &str,
    mentions: &[MessageMention],
) -> Result<(), MessagingError> {
    for (ordinal, mention) in mentions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO message_mentions( \
                message_id,ordinal,mention_kind,target_id,start_byte,end_byte \
             ) VALUES(?,?,?,?,?,?)",
        )
        .bind(message_id)
        .bind(ordinal as i64)
        .bind(mention.kind.as_str())
        .bind(&mention.target_id)
        .bind(mention.start_byte as i64)
        .bind(mention.end_byte as i64)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

fn normalize_channel_name(channel: &str) -> String {
    if channel.starts_with('#') {
        channel.to_owned()
    } else {
        format!("#{channel}")
    }
}

fn map_authorization_error(error: AuthorizationError) -> MessagingError {
    match error {
        AuthorizationError::Unavailable => MessagingError::Unavailable,
        AuthorizationError::Authentication(_) => MessagingError::Unauthenticated,
        AuthorizationError::Database(error) => MessagingError::Internal(error),
    }
}

async fn enforce_timeout(
    connection: &mut SqliteConnection,
    server_id: &str,
    user_id: &str,
) -> Result<(), MessagingError> {
    let timed_out: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
            SELECT 1 FROM server_members \
            WHERE server_id=? AND user_id=? AND timeout_until IS NOT NULL \
              AND timeout_until > datetime('now') \
         )",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_one(connection)
    .await?;
    if timed_out != 0 {
        return Err(MessagingError::Unavailable);
    }
    Ok(())
}

async fn enforce_rate_and_slow_mode(
    connection: &mut SqliteConnection,
    channel_id: &str,
    user_id: &str,
    slowmode_seconds: i64,
) -> Result<(), MessagingError> {
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages \
         WHERE channel_id=? AND sender_id=? \
           AND julianday(created_at) >= julianday('now', ?)",
    )
    .bind(channel_id)
    .bind(user_id)
    .bind(format!("-{RATE_WINDOW_SECONDS} seconds"))
    .fetch_one(&mut *connection)
    .await?;
    if recent >= RATE_WINDOW_MESSAGES {
        return Err(MessagingError::RateLimited);
    }
    if slowmode_seconds > 0 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT EXISTS( \
                SELECT 1 FROM messages WHERE channel_id=? AND sender_id=? \
                  AND julianday(created_at) > julianday('now', ?) \
             )",
        )
        .bind(channel_id)
        .bind(user_id)
        .bind(format!("-{slowmode_seconds} seconds"))
        .fetch_one(&mut *connection)
        .await?;
        if blocked != 0 {
            return Err(MessagingError::RateLimited);
        }
    }
    Ok(())
}

async fn enforce_automod(
    connection: &mut SqliteConnection,
    server_id: &str,
    actor_id: &str,
    operation_id: &str,
    content: &str,
) -> Result<(), MessagingError> {
    let rows = sqlx::query(
        "SELECT id,name,rule_type,config,action_type,timeout_duration_seconds \
         FROM automod_rules WHERE server_id=? AND enabled=1 ORDER BY created_at,id",
    )
    .bind(server_id)
    .fetch_all(&mut *connection)
    .await?;
    for row in rows {
        let rule_id: String = row.get(0);
        let rule_name: String = row.get(1);
        let rule_type: String = row.get(2);
        let config: serde_json::Value = serde_json::from_str(row.get::<&str, _>(3))
            .map_err(|_| MessagingError::DependencyUnavailable)?;
        let triggered = match rule_type.as_str() {
            "keyword" => config
                .get("words")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|words| {
                    let message_words = content
                        .to_lowercase()
                        .split(|character: char| !character.is_alphanumeric())
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    words.iter().any(|word| {
                        word.as_str().is_some_and(|word| {
                            message_words
                                .iter()
                                .any(|candidate| candidate == &word.to_lowercase())
                        })
                    })
                }),
            "mention_spam" => {
                let maximum = config
                    .get("max_mentions")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(5) as usize;
                content.matches('@').count() > maximum
            }
            "link_filter" => {
                config
                    .get("block_all")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                    && (content.contains("http://") || content.contains("https://"))
            }
            _ => false,
        };
        if triggered {
            let action_type: String = row.get(4);
            let timeout_seconds: Option<i64> = row.get(5);
            if action_type == "timeout" {
                let timeout_seconds = timeout_seconds
                    .filter(|seconds| (1..=2_419_200).contains(seconds))
                    .ok_or(MessagingError::DependencyUnavailable)?;
                sqlx::query(
                    "UPDATE server_members SET timeout_until=datetime('now',?) \
                     WHERE server_id=? AND user_id=?",
                )
                .bind(format!("+{timeout_seconds} seconds"))
                .bind(server_id)
                .bind(actor_id)
                .execute(&mut *connection)
                .await?;
            }
            let audit_id = format!("automod:{rule_id}:{actor_id}:{operation_id}");
            let details = serde_json::json!({
                "rule_id": rule_id,
                "rule_name": rule_name,
                "outcome": action_type,
            })
            .to_string();
            sqlx::query(
                "INSERT OR IGNORE INTO audit_log( \
                    id,server_id,actor_id,actor_username_snapshot,actor_avatar_snapshot, \
                    action_type,target_type,target_id,changes \
                 ) SELECT ?,?,?,COALESCE(u.username,?),u.avatar_url,?,?,?,? \
                   FROM (SELECT 1) LEFT JOIN users u ON u.id=?",
            )
            .bind(audit_id)
            .bind(server_id)
            .bind(actor_id)
            .bind(actor_id)
            .bind(if action_type == "flag" {
                "automod_flag"
            } else {
                "automod_reject"
            })
            .bind("user")
            .bind(actor_id)
            .bind(details)
            .bind(actor_id)
            .execute(&mut *connection)
            .await?;
            if action_type != "flag" {
                return Err(MessagingError::AutoModRejected(format!(
                    "message blocked by AutoMod rule: {rule_name}"
                )));
            }
        }
    }
    Ok(())
}

async fn validate_reply(
    connection: &mut SqliteConnection,
    conversation_id: &str,
    reply_to_id: Option<&str>,
) -> Result<(), MessagingError> {
    let Some(reply_to_id) = reply_to_id else {
        return Ok(());
    };
    let valid: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM messages \
         WHERE id=? AND conversation_id=? AND deleted_at IS NULL)",
    )
    .bind(reply_to_id)
    .bind(conversation_id)
    .fetch_one(connection)
    .await?;
    if valid == 0 {
        return Err(MessagingError::Unavailable);
    }
    Ok(())
}

async fn validate_attachments(
    connection: &mut SqliteConnection,
    user_id: &str,
    conversation_id: &str,
    attachment_ids: &[String],
) -> Result<(), MessagingError> {
    if attachment_ids.is_empty() {
        return Ok(());
    }
    let mut builder =
        sqlx::QueryBuilder::new("SELECT COUNT(*) FROM attachments WHERE uploader_id=");
    builder.push_bind(user_id);
    builder.push(" AND conversation_id=");
    builder.push_bind(conversation_id);
    builder.push(
        " AND media_state='ready' AND storage_backend='local' \
         AND storage_key IS NOT NULL AND message_id IS NULL AND id IN (",
    );
    let mut separated = builder.separated(",");
    for id in attachment_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let count: i64 = builder.build_query_scalar().fetch_one(connection).await?;
    if count != attachment_ids.len() as i64 {
        return Err(MessagingError::Unavailable);
    }
    Ok(())
}

async fn validate_mentions(
    connection: &mut SqliteConnection,
    server_id: &str,
    content: &str,
    mentions: &[MessageMention],
) -> Result<(), MessagingError> {
    let mut previous_end = 0;
    for mention in mentions {
        if mention.start_byte < previous_end
            || mention.end_byte > content.len()
            || !content.is_char_boundary(mention.start_byte)
            || !content.is_char_boundary(mention.end_byte)
            || mention.start_byte == mention.end_byte
        {
            return Err(MessagingError::InvalidInput("invalid mention range".into()));
        }
        previous_end = mention.end_byte;
        let exists = match mention.kind {
            MentionKind::Everyone => mention.target_id.is_none(),
            MentionKind::User => {
                let Some(target) = mention.target_id.as_deref() else {
                    return Err(MessagingError::InvalidInput(
                        "user mention requires a target".into(),
                    ));
                };
                sqlx::query_scalar::<_, i64>(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
                )
                .bind(server_id)
                .bind(target)
                .fetch_one(&mut *connection)
                .await?
                    != 0
            }
            MentionKind::Role => {
                let Some(target) = mention.target_id.as_deref() else {
                    return Err(MessagingError::InvalidInput(
                        "role mention requires a target".into(),
                    ));
                };
                sqlx::query_scalar::<_, i64>(
                    "SELECT EXISTS(SELECT 1 FROM roles WHERE server_id=? AND id=?)",
                )
                .bind(server_id)
                .bind(target)
                .fetch_one(&mut *connection)
                .await?
                    != 0
            }
        };
        if !exists {
            return Err(MessagingError::Unavailable);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::engine::permissions::DEFAULT_EVERYONE;

    async fn fixture() -> (SqlitePool, AuthService, Actor, MessagingService) {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username) VALUES('user','carmilla'),('other','laurelai')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES \
             ('user','user','canonical_id'),('other','other','canonical_id'), \
             ('laurelai','other','nickname')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('server','user','owner'),('server','other','member')",
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
        let auth = AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("user").await.unwrap().1;
        let service = MessagingService::new(pool.clone(), auth.clone(), 4000);
        (pool, auth, actor, service)
    }

    fn command<'a>(
        request_id: &'a str,
        client_message_id: &'a str,
        content: &'a str,
    ) -> SendMessageCommand<'a> {
        SendMessageCommand {
            request_id,
            client_message_id,
            operation_generation: None,
            conversation_id: None,
            server_id: "server",
            channel: "#general",
            content,
            content_format: ContentFormat::Markdown,
            reply_to_id: None,
            attachment_ids: &[],
            mentions: &[],
        }
    }

    fn command_in_generation<'a>(
        request_id: &'a str,
        client_message_id: &'a str,
        content: &'a str,
        operation_generation: &'a str,
    ) -> SendMessageCommand<'a> {
        SendMessageCommand {
            operation_generation: Some(operation_generation),
            ..command(request_id, client_message_id, content)
        }
    }

    #[tokio::test]
    async fn message_event_atomically_enqueues_subscribed_outgoing_webhook() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,is_private) VALUES \
             ('sibling','server','#sibling',0),('private','server','#private',1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhooks( \
                id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
             ) VALUES \
                ('hook','server','channel','Hook','outgoing','hash-main','https://example.com/hook','user','active'), \
                ('sibling-hook','server','sibling','Sibling','outgoing','hash-sibling','https://example.com/sibling','user','active'), \
                ('private-hook','server','private','Private','outgoing','hash-private','https://example.com/private','user','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhook_events(id,webhook_id,event_type) VALUES \
             ('subscription','hook','message_create'), \
             ('sibling-subscription','sibling-hook','message_create'), \
             ('private-subscription','private-hook','message_create')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let receipt = service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await
            .unwrap();
        let delivery: (String, String, String, i64) = sqlx::query_as(
            "SELECT d.state,j.state,j.destination_grant,d.event_sequence \
             FROM webhook_deliveries d JOIN external_jobs j ON j.id=d.external_job_id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(delivery.0, "pending");
        assert_eq!(delivery.1, "pending");
        assert_eq!(delivery.2, "webhook:hook:1");
        assert_eq!(delivery.3.to_string(), receipt.sequence);
        let payload: String = sqlx::query_scalar("SELECT payload_json FROM webhook_deliveries")
            .fetch_one(&pool)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["channel_id"], "channel");
        assert!(payload.get("event_sequence").is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM webhook_deliveries")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn webhook_enqueue_failure_rolls_back_canonical_message_and_event() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO webhooks( \
                id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
             ) VALUES('hook','server','channel','Hook','outgoing','hash', \
                'https://example.com/hook','user','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhook_events(id,webhook_id,event_type) \
             VALUES('subscription','hook','message_create')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_webhook_job BEFORE INSERT ON external_jobs \
             WHEN NEW.operation_type='webhook_delivery' \
             BEGIN SELECT RAISE(FAIL,'injected'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            service
                .send_channel_message(&actor, command("request", "client", "hello"))
                .await,
            Err(MessagingError::Internal(_))
        ));
        let counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM messages), \
                    (SELECT COUNT(*) FROM event_log), \
                    (SELECT COUNT(*) FROM external_jobs), \
                    (SELECT COUNT(*) FROM webhook_deliveries)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(counts, (0, 0, 0, 0));
    }

    #[tokio::test]
    async fn send_commits_message_receipt_event_and_outbox_before_returning() {
        let (pool, _, actor, service) = fixture().await;
        let receipt = service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await
            .unwrap();
        assert_eq!(receipt.sequence, "1");
        assert_eq!(receipt.event_sequence_internal, 1);
        assert!(!receipt.replayed);
        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT COUNT(*) FROM messages WHERE id=?), \
                (SELECT COUNT(*) FROM command_receipts WHERE canonical_message_id=?), \
                (SELECT COUNT(*) FROM event_log WHERE entity_id=?), \
                (SELECT COUNT(*) FROM delivery_outbox WHERE event_sequence=?)",
        )
        .bind(&receipt.message_id)
        .bind(&receipt.message_id)
        .bind(&receipt.message_id)
        .bind(receipt.event_sequence_internal as i64)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (1, 1, 1, 1));
    }

    #[tokio::test]
    async fn announcement_publish_is_deduplicated_and_lineage_propagates_after_unfollow() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query("UPDATE channels SET is_announcement=1 WHERE id='channel'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES('target-server','Target','user')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('target-server','user','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('target-channel','target-server','#news')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_follows(id,source_channel_id,target_channel_id,created_by) VALUES('follow','channel','target-channel','user')")
            .execute(&pool)
            .await
            .unwrap();
        let source = service
            .send_channel_message(&actor, command("send-request", "send-client", "original"))
            .await
            .unwrap();

        let first = service
            .publish_announcement(
                &actor,
                PublishAnnouncementCommand {
                    message_id: &source.message_id,
                },
            )
            .await
            .unwrap();
        let retry = service
            .publish_announcement(
                &actor,
                PublishAnnouncementCommand {
                    message_id: &source.message_id,
                },
            )
            .await
            .unwrap();
        assert_eq!(first, retry);
        assert_eq!(first.len(), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM announcement_publications")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );

        sqlx::query("DELETE FROM channel_follows WHERE id='follow'")
            .execute(&pool)
            .await
            .unwrap();
        service
            .edit_message(
                &actor,
                EditMessageCommand {
                    request_id: "edit-request",
                    client_message_id: "edit-client",
                    operation_generation: None,
                    message_id: &source.message_id,
                    content: "corrected",
                    content_format: ContentFormat::Markdown,
                    mentions: &[],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id=?")
                .bind(&first[0].target_message_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "corrected"
        );
        service
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: "delete-request",
                    client_message_id: "delete-client",
                    operation_generation: None,
                    message_id: &source.message_id,
                },
            )
            .await
            .unwrap();
        let state: (String, i64, i64) = sqlx::query_as(
            "SELECT ap.state,ap.source_version,(m.deleted_at IS NOT NULL) \
             FROM announcement_publications ap JOIN messages m ON m.id=ap.target_message_id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, ("deleted".into(), 3, 1));
    }

    #[tokio::test]
    async fn announcement_lineage_survives_reopen_without_duplicate_destination() {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!("sqlite://{}", directory.path().join("concord.db").display());
        let pool = create_pool(&database_url).await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('user','carmilla')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('user','user','canonical_id')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Source','user'),('target-server','Target','user')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','user','owner'),('target-server','user','owner')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
            .bind(DEFAULT_EVERYONE.bits() as i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,is_announcement) VALUES('channel','server','#general',1),('target-channel','target-server','#news',0)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_follows(id,source_channel_id,target_channel_id,created_by) VALUES('follow','channel','target-channel','user')")
            .execute(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("user").await.unwrap().1;
        let service = MessagingService::new(pool.clone(), auth, 4000);
        let source = service
            .send_channel_message(&actor, command("send", "client", "original"))
            .await
            .unwrap();
        let published = service
            .publish_announcement(
                &actor,
                PublishAnnouncementCommand {
                    message_id: &source.message_id,
                },
            )
            .await
            .unwrap();
        assert_eq!(published.len(), 1);
        let target_message_id = published[0].target_message_id.clone();
        drop(service);
        pool.close().await;

        let pool = create_pool(&database_url).await.unwrap();
        let auth = AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("user").await.unwrap().1;
        let service = MessagingService::new(pool.clone(), auth, 4000);
        let replay = service
            .publish_announcement(
                &actor,
                PublishAnnouncementCommand {
                    message_id: &source.message_id,
                },
            )
            .await
            .unwrap();
        assert_eq!(replay[0].target_message_id, target_message_id);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM announcement_publications WHERE source_message_id=?"
            )
            .bind(&source.message_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        sqlx::query("DELETE FROM channel_follows WHERE id='follow'")
            .execute(&pool)
            .await
            .unwrap();
        service
            .edit_message(
                &actor,
                EditMessageCommand {
                    request_id: "edit",
                    client_message_id: "edit-client",
                    operation_generation: None,
                    message_id: &source.message_id,
                    content: "corrected",
                    content_format: ContentFormat::Markdown,
                    mentions: &[],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id=?")
                .bind(&target_message_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "corrected"
        );
        drop(service);
        pool.close().await;

        let pool = create_pool(&database_url).await.unwrap();
        let auth = AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("user").await.unwrap().1;
        let service = MessagingService::new(pool.clone(), auth, 4000);
        service
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: "delete",
                    client_message_id: "delete-client",
                    operation_generation: None,
                    message_id: &source.message_id,
                },
            )
            .await
            .unwrap();
        let state: (String, i64, i64, i64) = sqlx::query_as("SELECT ap.state,ap.source_version,(m.deleted_at IS NOT NULL),(SELECT count(*) FROM announcement_publications WHERE source_message_id=?) FROM announcement_publications ap JOIN messages m ON m.id=ap.target_message_id WHERE ap.source_message_id=?")
            .bind(&source.message_id).bind(&source.message_id).fetch_one(&pool).await.unwrap();
        assert_eq!(state, ("deleted".into(), 3, 1, 1));
    }

    #[tokio::test]
    async fn identical_retry_returns_canonical_receipt_and_conflict_is_rejected() {
        let (pool, _, actor, service) = fixture().await;
        let original = service
            .send_channel_message(&actor, command("request-1", "client", "hello"))
            .await
            .unwrap();
        let retry = service
            .send_channel_message(&actor, command("request-2", "client", "hello"))
            .await
            .unwrap();
        assert!(retry.replayed);
        assert_eq!(retry.request_id, "request-2");
        assert_eq!(retry.message_id, original.message_id);
        assert_eq!(retry.sequence, original.sequence);
        assert!(matches!(
            service
                .send_channel_message(&actor, command("request-3", "client", "different"))
                .await,
            Err(MessagingError::IdempotencyConflict)
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn retained_receipt_wins_across_database_restore_and_operation_rollover() {
        let (pool, _, actor, service) = fixture().await;
        let original_generation: String = sqlx::query_scalar(
            "SELECT current_generation FROM operation_generation_state WHERE singleton=1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let original = service
            .send_channel_message(
                &actor,
                command_in_generation("request-1", "stable-client", "hello", &original_generation),
            )
            .await
            .unwrap();
        sqlx::query(
            "UPDATE database_metadata SET generation='restored-database' WHERE singleton=1",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO operation_generations(generation,issued_at,expires_at) \
             VALUES('next-operation-generation',unixepoch(),unixepoch()+604800)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE operation_generation_state SET current_generation='next-operation-generation' \
             WHERE singleton=1",
        )
        .execute(&pool)
        .await
        .unwrap();

        let retry = service
            .send_channel_message(
                &actor,
                command_in_generation("request-2", "stable-client", "hello", &original_generation),
            )
            .await
            .unwrap();
        assert!(retry.replayed);
        assert_eq!(retry.message_id, original.message_id);
        assert!(matches!(
            service
                .send_channel_message(
                    &actor,
                    command_in_generation(
                        "request-3",
                        "stable-client",
                        "different",
                        "next-operation-generation",
                    ),
                )
                .await,
            Err(MessagingError::IdempotencyConflict)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn expired_generation_rejects_an_operation_without_a_retained_receipt() {
        let (pool, _, actor, service) = fixture().await;
        let generation: String = sqlx::query_scalar(
            "SELECT current_generation FROM operation_generation_state WHERE singleton=1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE operation_generations SET issued_at=unixepoch()-2,expires_at=unixepoch()-1 \
             WHERE generation=?",
        )
        .bind(&generation)
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            service
                .send_channel_message(
                    &actor,
                    command_in_generation("request", "missing-client", "hello", &generation),
                )
                .await,
            Err(MessagingError::OperationGenerationExpired)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn forced_event_failure_rolls_back_message_sequence_and_receipt() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "CREATE TRIGGER fail_event BEFORE INSERT ON event_log \
             BEGIN SELECT RAISE(ABORT,'forced event failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, MessagingError::Internal(source) if source.to_string().contains("forced event failure")),
            "fault injection did not reach event insertion: {error:?}"
        );
        let state: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT COUNT(*) FROM messages), \
                (SELECT COUNT(*) FROM command_receipts), \
                (SELECT next_message_sequence FROM conversations WHERE channel_id='channel')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 0, 0));
    }

    #[tokio::test]
    async fn interaction_response_rolls_back_consumption_when_message_event_fails() {
        let (pool, auth, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO interactions
             (id,interaction_type,user_id,server_id,channel_id,data_json,
              application_user_id,expires_at,response_state)
             VALUES('interaction','slash_command','other','server','channel','{}',
                    'user',datetime('now','+5 minutes'),'pending')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_interaction_event BEFORE INSERT ON event_log
             BEGIN SELECT RAISE(ABORT,'forced interaction event failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let error = service
            .respond_to_interaction_public(
                &actor,
                "interaction",
                command(
                    "interaction-request",
                    "interaction:interaction:response:1",
                    "hello",
                ),
                Some(r##"[{"title":"Result","url":"https://example.test/result","color":"#5865f2"}]"##),
                Some(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, MessagingError::Internal(source) if source.to_string().contains("forced interaction event failure"))
        );
        let state: (String, i64, i64) = sqlx::query_as(
            "SELECT response_state,
                    (SELECT COUNT(*) FROM messages),
                    (SELECT COUNT(*) FROM command_receipts)
             FROM interactions WHERE id='interaction'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, ("pending".into(), 0, 0));

        sqlx::query("DROP TRIGGER fail_interaction_event")
            .execute(&pool)
            .await
            .unwrap();
        let receipt = service
            .respond_to_interaction_public(
                &actor,
                "interaction",
                command(
                    "interaction-retry",
                    "interaction:interaction:response:1",
                    "hello",
                ),
                Some(r##"[{"title":"Result","url":"https://example.test/result","color":"#5865f2"}]"##),
                Some(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#),
            )
            .await
            .unwrap();
        let committed: (String, String, String, String) = sqlx::query_as(
            "SELECT i.response_state,i.response_message_id,m.rich_embeds_json,m.components_json
             FROM interactions i JOIN messages m ON m.id=i.response_message_id
             WHERE i.id='interaction'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(committed.0, "responded");
        assert_eq!(committed.1, receipt.message_id);
        assert!(committed.2.contains("Result"));
        assert!(committed.3.contains("confirm"));
        let conversation_id: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let snapshot = crate::engine::replay::ReplayService::new(pool, auth, "replay-secret")
            .snapshot(&actor, &[conversation_id])
            .await
            .unwrap();
        let projected = snapshot
            .messages
            .iter()
            .find(|message| message.message_id == receipt.message_id)
            .unwrap();
        assert_eq!(
            projected.rich_embeds.as_ref().unwrap()[0].title.as_deref(),
            Some("Result")
        );
        assert!(matches!(
            projected.components.as_ref().unwrap()[0],
            crate::engine::events::MessageComponent::ActionRow { .. }
        ));
    }

    #[tokio::test]
    async fn forced_receipt_failure_rolls_back_message_event_and_outbox() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "CREATE TRIGGER fail_receipt BEFORE INSERT ON command_receipts \
             BEGIN SELECT RAISE(ABORT,'forced receipt failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            service
                .send_channel_message(&actor, command("request", "client", "hello"))
                .await,
            Err(MessagingError::Internal(_))
        ));
        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM messages), \
                    (SELECT COUNT(*) FROM event_log), \
                    (SELECT COUNT(*) FROM delivery_outbox), \
                    (SELECT next_message_sequence FROM conversations WHERE channel_id='channel')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 0, 0, 0));
    }

    #[tokio::test]
    async fn forced_attachment_link_failure_rolls_back_message_and_claim() {
        let (pool, _, actor, service) = fixture().await;
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO attachments( \
                 id,uploader_id,filename,original_filename,content_type,file_size,conversation_id, \
                 media_state,storage_backend,storage_key,reserved_bytes \
             ) VALUES('attachment','user','file','file','text/plain',4,?,'ready','local','key',4)",
        )
        .bind(conversation)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_attachment BEFORE UPDATE ON attachments \
             WHEN NEW.media_state='attached' \
             BEGIN SELECT RAISE(ABORT,'forced attachment link failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let attachments = vec!["attachment".to_owned()];
        let mut send = command("request", "client", "hello");
        send.attachment_ids = &attachments;
        assert!(matches!(
            service.send_channel_message(&actor, send).await,
            Err(MessagingError::Internal(_))
        ));
        let state: (i64, Option<String>, String) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM messages),message_id,media_state \
             FROM attachments WHERE id='attachment'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, None, "ready".into()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_sends_allocate_unique_monotonic_sequences() {
        let (_, _, actor, service) = fixture().await;
        let first = {
            let actor = actor.clone();
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .send_channel_message(&actor, command("r1", "c1", "one"))
                    .await
                    .unwrap()
            })
        };
        let second = {
            let actor = actor.clone();
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .send_channel_message(&actor, command("r2", "c2", "two"))
                    .await
                    .unwrap()
            })
        };
        let mut sequences = [
            first.await.unwrap().sequence.parse::<u64>().unwrap(),
            second.await.unwrap().sequence.parse::<u64>().unwrap(),
        ];
        sequences.sort_unstable();
        assert_eq!(sequences, [1, 2]);
    }

    #[tokio::test]
    async fn pool_exhaustion_is_bounded_by_the_overall_admission_deadline() {
        let (pool, _, actor, service) = fixture().await;
        let mut held = Vec::new();
        for _ in 0..5 {
            held.push(pool.acquire().await.unwrap());
        }
        let started = std::time::Instant::now();
        let result = service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await;
        assert!(matches!(result, Err(MessagingError::DependencyUnavailable)));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        drop(held);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn locked_database_timeout_cancels_cleanly_and_later_send_succeeds() {
        let before = crate::runtime_metrics::snapshot();
        let database_index = crate::runtime_metrics::Operation::DatabaseWrite as usize;
        let (pool, _, actor, service) = fixture().await;
        let lock = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let result = service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await;
        assert!(matches!(result, Err(MessagingError::DependencyUnavailable)));
        lock.rollback().await.unwrap();
        let receipt = service
            .send_channel_message(&actor, command("request", "client", "hello"))
            .await
            .unwrap();
        assert_eq!(receipt.sequence, "1");
        let after = crate::runtime_metrics::snapshot();
        assert!(after.failed[database_index] > before.failed[database_index]);
        assert!(after.succeeded[database_index] > before.succeeded[database_index]);
    }

    #[test]
    fn entity_tuple_ids_are_unambiguous_with_colon_bearing_fields() {
        assert_ne!(
            reaction_entity_id("message:part", "did:plc:user", "emoji"),
            reaction_entity_id("message", "part:did:plc:user", "emoji")
        );
        assert_ne!(
            read_entity_id("did:plc:user", "direct:a:b"),
            read_entity_id("did", "plc:user:direct:a:b")
        );
        assert_ne!(
            reaction_entity_id("same", "tuple", "value"),
            read_entity_id("same", "tuple:value")
        );
    }

    #[tokio::test]
    async fn direct_send_resolves_alias_and_commits_one_canonical_conversation() {
        let (pool, _, actor, service) = fixture().await;
        let receipt = service
            .send_direct_message(
                &actor,
                SendDirectMessageCommand {
                    request_id: "direct-1",
                    client_message_id: "direct-client",
                    operation_generation: None,
                    recipient: "LaUrElAi",
                    content: "hello",
                    content_format: ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
            )
            .await
            .unwrap();
        let row: (String, String, String, i64) = sqlx::query_as(
            "SELECT m.conversation_id,m.target_user_id,m.content, \
                    (SELECT COUNT(*) FROM conversation_participants cp \
                     WHERE cp.conversation_id=m.conversation_id) \
             FROM messages m WHERE m.id=?",
        )
        .bind(&receipt.message_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.0.starts_with("direct:"));
        assert_eq!(row.1, "other");
        assert_eq!(row.2, "hello");
        assert_eq!(row.3, 2);

        let retry = service
            .send_direct_message(
                &actor,
                SendDirectMessageCommand {
                    request_id: "direct-2",
                    client_message_id: "direct-client",
                    operation_generation: None,
                    recipient: "other",
                    content: "hello",
                    content_format: ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
            )
            .await
            .unwrap();
        assert!(retry.replayed);
        assert_eq!(retry.message_id, receipt.message_id);
    }

    #[tokio::test]
    async fn direct_send_reuses_existing_opaque_pair_without_creating_an_orphan() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query("INSERT INTO conversations(id,kind) VALUES('opaque-direct','direct')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO direct_conversation_pairs( \
                 conversation_id,lower_user_id,upper_user_id \
             ) VALUES('opaque-direct','other','user')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversation_participants(conversation_id,user_id) \
             VALUES('opaque-direct','other'),('opaque-direct','user')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let receipt = service
            .send_direct_message(
                &actor,
                SendDirectMessageCommand {
                    request_id: "opaque-direct-request",
                    client_message_id: "opaque-direct-client",
                    operation_generation: None,
                    recipient: "other",
                    content: "existing history",
                    content_format: ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
            )
            .await
            .unwrap();

        let message_conversation: String =
            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                .bind(&receipt.message_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let direct_conversations: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE kind='direct'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(message_conversation, "opaque-direct");
        assert_eq!(direct_conversations, 1);
    }

    #[tokio::test]
    async fn direct_send_block_rolls_back_new_conversation_and_message() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO user_blocks(blocker_user_id,blocked_user_id) VALUES('other','user')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let result = service
            .send_direct_message(
                &actor,
                SendDirectMessageCommand {
                    request_id: "direct",
                    client_message_id: "direct-client",
                    operation_generation: None,
                    recipient: "other",
                    content: "hello",
                    content_format: ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
            )
            .await;
        assert!(matches!(result, Err(MessagingError::Unavailable)));
        let state: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM conversations WHERE kind='direct'), \
                    (SELECT COUNT(*) FROM direct_conversation_pairs), \
                    (SELECT COUNT(*) FROM messages WHERE channel_id IS NULL)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 0, 0));
    }

    #[tokio::test]
    async fn delete_retry_is_canonical_and_new_reaction_on_tombstone_is_rejected() {
        let (pool, _, actor, service) = fixture().await;
        let sent = service
            .send_channel_message(&actor, command("send", "send-client", "hello"))
            .await
            .unwrap();
        let delete = EntityCommand {
            request_id: "delete-1",
            client_message_id: "delete-client",
            operation_generation: None,
            message_id: &sent.message_id,
        };
        let original = service
            .delete_message(&actor, delete)
            .await
            .unwrap()
            .receipt;
        let retry = service
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: "delete-2",
                    client_message_id: "delete-client",
                    operation_generation: None,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap()
            .receipt;
        assert!(retry.replayed);
        assert_eq!(retry.request_id, "delete-2");
        assert_eq!(
            retry.event_sequence_internal,
            original.event_sequence_internal
        );

        let reaction = service
            .change_reaction(
                &actor,
                ReactionCommand {
                    request_id: "reaction",
                    client_message_id: "reaction-client",
                    operation_generation: None,
                    message_id: &sent.message_id,
                    emoji: "heart",
                },
                true,
            )
            .await;
        assert!(matches!(reaction, Err(MessagingError::Unavailable)));
        let reaction_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reactions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(reaction_count, 0);
    }

    #[tokio::test]
    async fn deleting_a_channel_message_does_not_reset_slow_mode() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query("UPDATE channels SET slowmode_seconds=60 WHERE id='channel'")
            .execute(&pool)
            .await
            .unwrap();
        let sent = service
            .send_channel_message(&actor, command("slow-send", "slow-client", "one"))
            .await
            .unwrap();
        service
            .delete_message(
                &actor,
                EntityCommand {
                    request_id: "slow-delete",
                    client_message_id: "slow-delete-client",
                    operation_generation: None,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            service
                .send_channel_message(&actor, command("slow-send-2", "slow-client-2", "two"))
                .await,
            Err(MessagingError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn deleted_channel_messages_still_consume_the_send_rate_budget() {
        let (_, _, actor, service) = fixture().await;
        for index in 0..RATE_WINDOW_MESSAGES {
            let request = format!("rate-send-{index}");
            let client = format!("rate-client-{index}");
            let sent = service
                .send_channel_message(&actor, command(&request, &client, "message"))
                .await
                .unwrap();
            let delete_request = format!("rate-delete-{index}");
            let delete_client = format!("rate-delete-client-{index}");
            service
                .delete_message(
                    &actor,
                    EntityCommand {
                        request_id: &delete_request,
                        client_message_id: &delete_client,
                        operation_generation: None,
                        message_id: &sent.message_id,
                    },
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            service
                .send_channel_message(&actor, command("rate-over", "rate-over-client", "blocked"))
                .await,
            Err(MessagingError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn deleted_direct_messages_still_consume_the_send_rate_budget() {
        let (_, _, actor, service) = fixture().await;
        for index in 0..RATE_WINDOW_MESSAGES {
            let request = format!("dm-send-{index}");
            let client = format!("dm-client-{index}");
            let sent = service
                .send_direct_message(
                    &actor,
                    SendDirectMessageCommand {
                        request_id: &request,
                        client_message_id: &client,
                        operation_generation: None,
                        recipient: "other",
                        content: "message",
                        content_format: ContentFormat::Plain,
                        reply_to_id: None,
                        attachment_ids: &[],
                    },
                )
                .await
                .unwrap();
            let delete_request = format!("dm-delete-{index}");
            let delete_client = format!("dm-delete-client-{index}");
            service
                .delete_message(
                    &actor,
                    EntityCommand {
                        request_id: &delete_request,
                        client_message_id: &delete_client,
                        operation_generation: None,
                        message_id: &sent.message_id,
                    },
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            service
                .send_direct_message(
                    &actor,
                    SendDirectMessageCommand {
                        request_id: "dm-over",
                        client_message_id: "dm-over-client",
                        operation_generation: None,
                        recipient: "other",
                        content: "blocked",
                        content_format: ContentFormat::Plain,
                        reply_to_id: None,
                        attachment_ids: &[],
                    },
                )
                .await,
            Err(MessagingError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn read_state_never_moves_backwards() {
        let (pool, _, actor, service) = fixture().await;
        let first = service
            .send_channel_message(&actor, command("send-1", "send-client-1", "one"))
            .await
            .unwrap();
        let second = service
            .send_channel_message(&actor, command("send-2", "send-client-2", "two"))
            .await
            .unwrap();
        let conversation_id: String =
            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                .bind(&first.message_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        for (request_id, client_message_id, message_id) in [
            ("read-2", "read-client-2", second.message_id.as_str()),
            ("read-1", "read-client-1", first.message_id.as_str()),
            (
                "read-2-again",
                "read-client-2-again",
                second.message_id.as_str(),
            ),
        ] {
            service
                .mark_read(
                    &actor,
                    ReadCommand {
                        request_id,
                        client_message_id,
                        operation_generation: None,
                        conversation_id: &conversation_id,
                        message_id,
                    },
                )
                .await
                .unwrap();
        }
        let state: (String, i64) = sqlx::query_as(
            "SELECT last_read_message_id,conversation_sequence FROM read_states \
             WHERE user_id='user' AND channel_id='channel'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (second.message_id, 2));
        let durable_state: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT COUNT(*) FROM event_log WHERE event_kind='read_advanced'), \
                (SELECT version FROM entity_versions WHERE entity_type='read_state'), \
                (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='read')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(durable_state, (1, 1, 3));
    }

    #[tokio::test]
    async fn already_read_legacy_state_succeeds_without_creating_durable_churn() {
        let (pool, _, actor, service) = fixture().await;
        let sent = service
            .send_channel_message(&actor, command("send", "send-client", "one"))
            .await
            .unwrap();
        let conversation_id: String =
            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                .bind(&sent.message_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO read_states( \
                 user_id,channel_id,last_read_message_id,conversation_sequence \
             ) VALUES('user','channel',?,1)",
        )
        .bind(&sent.message_id)
        .execute(&pool)
        .await
        .unwrap();

        service
            .mark_read(
                &actor,
                ReadCommand {
                    request_id: "legacy-read",
                    client_message_id: "legacy-read-client",
                    operation_generation: None,
                    conversation_id: &conversation_id,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
        let state: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT COUNT(*) FROM event_log WHERE entity_type='read_state'), \
                (SELECT COUNT(*) FROM entity_versions WHERE entity_type='read_state'), \
                (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='read')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 0, 1));
    }

    #[tokio::test]
    async fn already_read_state_succeeds_after_transport_event_pruning() {
        let (pool, _, actor, service) = fixture().await;
        let sent = service
            .send_channel_message(&actor, command("send", "send-client", "one"))
            .await
            .unwrap();
        let conversation_id: String =
            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                .bind(&sent.message_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        service
            .mark_read(
                &actor,
                ReadCommand {
                    request_id: "read",
                    client_message_id: "read-client",
                    operation_generation: None,
                    conversation_id: &conversation_id,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
        sqlx::query(
            "DELETE FROM delivery_outbox WHERE event_sequence IN ( \
                 SELECT event_sequence FROM event_log WHERE entity_type='read_state' \
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM event_log WHERE entity_type='read_state'")
            .execute(&pool)
            .await
            .unwrap();

        service
            .mark_read(
                &actor,
                ReadCommand {
                    request_id: "read-again",
                    client_message_id: "read-client-again",
                    operation_generation: None,
                    conversation_id: &conversation_id,
                    message_id: &sent.message_id,
                },
            )
            .await
            .unwrap();
        let state: (i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT COUNT(*) FROM event_log WHERE entity_type='read_state'), \
                (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='read')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 2));
    }

    #[tokio::test]
    async fn automod_rejected_edit_preserves_message_and_has_no_receipt_or_event() {
        let (pool, _, actor, service) = fixture().await;
        let sent = service
            .send_channel_message(&actor, command("send", "send-client", "allowed"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO automod_rules(id,server_id,name,rule_type,config) \
             VALUES('rule','server','blocked','keyword','{\"words\":[\"forbidden\"]}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let result = service
            .edit_message(
                &actor,
                EditMessageCommand {
                    request_id: "edit",
                    client_message_id: "edit-client",
                    operation_generation: None,
                    message_id: &sent.message_id,
                    content: "forbidden",
                    content_format: ContentFormat::Plain,
                    mentions: &[],
                },
            )
            .await;
        assert!(matches!(result, Err(MessagingError::AutoModRejected(_))));
        let state: (String, i64, i64, i64) = sqlx::query_as(
            "SELECT content, \
                (SELECT COUNT(*) FROM event_log WHERE event_kind='message_edited'), \
                (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='edit'), \
                (SELECT COUNT(*) FROM audit_log WHERE action_type='automod_reject') \
             FROM messages WHERE id=?",
        )
        .bind(&sent.message_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, ("allowed".into(), 0, 0, 1));
    }

    #[tokio::test]
    async fn automod_timeout_reject_commits_timeout_and_deduplicated_audit_only() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO automod_rules( \
                id,server_id,name,rule_type,config,action_type,timeout_duration_seconds \
             ) VALUES('rule','server','No spam','keyword', \
                      '{\"words\":[\"spam\"]}','timeout',60)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let result = service
            .send_channel_message(&actor, command("send-one", "same-client", "spam"))
            .await;
        assert!(matches!(result, Err(MessagingError::AutoModRejected(_))));
        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM messages WHERE content='spam'), \
                (SELECT count(*) FROM command_receipts WHERE client_message_id='same-client'), \
                (SELECT count(*) FROM audit_log WHERE action_type='automod_reject'), \
                (SELECT count(*) FROM server_members \
                 WHERE server_id='server' AND user_id='user' AND timeout_until>datetime('now'))",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 0, 1, 1));
    }

    #[tokio::test]
    async fn automod_flag_accepts_message_without_storing_content_in_audit() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO automod_rules( \
                id,server_id,name,rule_type,config,action_type \
             ) VALUES('rule','server','Review links','link_filter', \
                      '{\"block_all\":true}','flag')",
        )
        .execute(&pool)
        .await
        .unwrap();
        service
            .send_channel_message(
                &actor,
                command("flag-send", "flag-client", "https://private.example/path"),
            )
            .await
            .unwrap();
        let details: String =
            sqlx::query_scalar("SELECT changes FROM audit_log WHERE action_type='automod_flag'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(details.contains("Review links"));
        assert!(!details.contains("private.example"));
    }

    #[tokio::test]
    async fn mutation_event_failure_rolls_back_projection_and_receipt() {
        let (pool, _, actor, service) = fixture().await;
        let sent = service
            .send_channel_message(&actor, command("send", "send-client", "before"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_edit_event BEFORE INSERT ON event_log \
             WHEN NEW.event_kind='message_edited' \
             BEGIN SELECT RAISE(ABORT,'forced edit event failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let result = service
            .edit_message(
                &actor,
                EditMessageCommand {
                    request_id: "edit",
                    client_message_id: "edit-client",
                    operation_generation: None,
                    message_id: &sent.message_id,
                    content: "after",
                    content_format: ContentFormat::Plain,
                    mentions: &[],
                },
            )
            .await;
        assert!(matches!(result, Err(MessagingError::Internal(_))));
        let state: (String, i64, i64) = sqlx::query_as(
            "SELECT content,entity_version, \
                (SELECT COUNT(*) FROM command_receipts WHERE operation_kind='edit') \
             FROM messages WHERE id=?",
        )
        .bind(&sent.message_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, ("before".into(), 1, 0));
    }
}
