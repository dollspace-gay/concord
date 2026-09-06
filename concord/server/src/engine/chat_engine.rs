use std::sync::{Arc, OnceLock};
use std::time::Instant;

use chrono::Utc;
use dashmap::DashMap;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::authorization::ChannelAction;
use super::channel::ChannelState;
use super::events::{
    AuditLogEntry, AutomodRuleInfo, BanInfo, BookmarkInfo, BotTokenInfo, CategoryInfo,
    ChannelFollowInfo, ChannelInfo, ChannelPositionInfo, ChatEvent, ConnectionId,
    DirectConversationInfo, EventInfo, HistoryMessage, InteractionInfo, InteractionResponseData,
    InviteInfo, MemberInfo, MemberRoleInfo, OAuth2AppInfo, PinnedMessageInfo, ReactionGroup,
    ReplyInfo, RoleInfo, RsvpInfo, ServerCommunityInfo, ServerInfo, SlashCommandInfo,
    SlashCommandOption, TemplateInfo, ThreadInfo, WebhookInfo,
};
use super::ids::{ChannelId, ServerId};

pub struct CreateAutomodRuleRequest<'a> {
    pub server_id: &'a str,
    pub name: &'a str,
    pub rule_type: &'a str,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

pub struct UpdateAutomodRuleRequest<'a> {
    pub rule_id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

pub struct CreateServerEventRequest<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub channel_id: Option<&'a str>,
    pub start_time: &'a str,
    pub end_time: Option<&'a str>,
    pub image_url: Option<&'a str>,
    pub created_by: &'a str,
}

#[derive(Debug)]
pub enum AtprotoPublicationError {
    Unavailable,
    Authentication,
    DependencyUnavailable,
    Database(sqlx::Error),
}

impl std::fmt::Display for AtprotoPublicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("publication unavailable"),
            Self::Authentication => formatter.write_str("publication authentication required"),
            Self::DependencyUnavailable | Self::Database(_) => {
                formatter.write_str("publication dependency unavailable")
            }
        }
    }
}

impl std::error::Error for AtprotoPublicationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<crate::db::queries::atproto::PublicationRequestError> for AtprotoPublicationError {
    fn from(error: crate::db::queries::atproto::PublicationRequestError) -> Self {
        match error {
            crate::db::queries::atproto::PublicationRequestError::Unavailable => Self::Unavailable,
            crate::db::queries::atproto::PublicationRequestError::Authentication => {
                Self::Authentication
            }
            crate::db::queries::atproto::PublicationRequestError::DependencyUnavailable => {
                Self::DependencyUnavailable
            }
            crate::db::queries::atproto::PublicationRequestError::Database(error) => {
                Self::Database(error)
            }
        }
    }
}

fn referenced_server_id(value: &str) -> Result<ServerId, String> {
    ServerId::from_stored(value.to_owned())
        .map_err(|_| "INVALID_INPUT: invalid server id".to_owned())
}

fn referenced_channel_id(value: &str) -> Result<ChannelId, String> {
    ChannelId::from_stored(value.to_owned())
        .map_err(|_| "INVALID_INPUT: invalid channel id".to_owned())
}

fn find_message_component<'a>(
    components: &'a [super::events::MessageComponent],
    custom_id: &str,
) -> Option<&'a super::events::MessageComponent> {
    for component in components {
        match component {
            super::events::MessageComponent::ActionRow { components } => {
                if let Some(found) = find_message_component(components, custom_id) {
                    return Some(found);
                }
            }
            super::events::MessageComponent::Button {
                custom_id: candidate,
                ..
            }
            | super::events::MessageComponent::SelectMenu {
                custom_id: candidate,
                ..
            } if candidate == custom_id => return Some(component),
            _ => {}
        }
    }
    None
}

fn safe_embed_url(value: &str) -> bool {
    if value.len() > 2_048 {
        return false;
    }
    reqwest::Url::parse(value).ok().is_some_and(|url| {
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        let host = url
            .host_str()
            .unwrap()
            .trim_matches(['[', ']'])
            .to_ascii_lowercase();
        if host == "localhost"
            || host.ends_with(".localhost")
            || host.ends_with(".local")
            || host.ends_with(".internal")
        {
            return false;
        }
        match host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(address)) => {
                !(address.is_private()
                    || address.is_loopback()
                    || address.is_link_local()
                    || address.is_unspecified()
                    || address.is_multicast())
            }
            Ok(std::net::IpAddr::V6(address)) => {
                !(address.is_loopback()
                    || address.is_unspecified()
                    || address.is_multicast()
                    || (address.segments()[0] & 0xfe00) == 0xfc00
                    || (address.segments()[0] & 0xffc0) == 0xfe80)
            }
            Err(_) => true,
        }
    })
}

fn validate_rich_interaction_response(
    embeds: Option<&[super::events::RichEmbedInfo]>,
    components: Option<&[super::events::MessageComponent]>,
) -> Result<(), String> {
    if let Some(embeds) = embeds {
        for embed in embeds {
            if embed.title.as_ref().is_some_and(|value| value.len() > 256)
                || embed
                    .description
                    .as_ref()
                    .is_some_and(|value| value.len() > 4096)
                || embed.color.as_ref().is_some_and(|value| {
                    value.len() != 7
                        || !value.starts_with('#')
                        || !value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                || embed.fields.as_ref().is_some_and(|fields| {
                    fields.len() > 25
                        || fields
                            .iter()
                            .any(|field| field.name.len() > 256 || field.value.len() > 1024)
                })
                || embed
                    .footer
                    .as_ref()
                    .is_some_and(|footer| footer.text.len() > 2048)
                || embed
                    .author
                    .as_ref()
                    .is_some_and(|author| author.name.len() > 256)
            {
                return Err("Invalid interaction embed".into());
            }
            for url in [
                embed.url.as_deref(),
                embed.image_url.as_deref(),
                embed.thumbnail_url.as_deref(),
                embed
                    .footer
                    .as_ref()
                    .and_then(|footer| footer.icon_url.as_deref()),
                embed
                    .author
                    .as_ref()
                    .and_then(|author| author.url.as_deref()),
                embed
                    .author
                    .as_ref()
                    .and_then(|author| author.icon_url.as_deref()),
            ]
            .into_iter()
            .flatten()
            {
                if !safe_embed_url(url) {
                    return Err("Interaction embed URL must use HTTPS".into());
                }
            }
        }
    }
    let mut custom_ids = std::collections::HashSet::new();
    if let Some(components) = components {
        validate_message_components(components, true, &mut custom_ids)?;
    }
    Ok(())
}

fn validate_message_components(
    components: &[super::events::MessageComponent],
    top_level: bool,
    custom_ids: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    for component in components {
        match component {
            super::events::MessageComponent::ActionRow { components } => {
                if !top_level || components.is_empty() || components.len() > 5 {
                    return Err("Invalid interaction component layout".into());
                }
                validate_message_components(components, false, custom_ids)?;
            }
            super::events::MessageComponent::Button {
                custom_id,
                label,
                style,
                emoji,
                ..
            } => {
                if top_level
                    || custom_id.is_empty()
                    || custom_id.len() > 100
                    || label.is_empty()
                    || label.len() > 80
                    || emoji.as_ref().is_some_and(|value| value.len() > 64)
                    || !matches!(
                        style.as_str(),
                        "primary" | "secondary" | "success" | "danger"
                    )
                    || !custom_ids.insert(custom_id.clone())
                {
                    return Err("Invalid interaction button".into());
                }
            }
            super::events::MessageComponent::SelectMenu {
                custom_id,
                placeholder,
                options,
                min_values,
                max_values,
            } => {
                let unique_values: std::collections::HashSet<_> =
                    options.iter().map(|option| option.value.as_str()).collect();
                if top_level
                    || custom_id.is_empty()
                    || custom_id.len() > 100
                    || placeholder.as_ref().is_some_and(|value| value.len() > 150)
                    || options.is_empty()
                    || options.len() > 25
                    || unique_values.len() != options.len()
                    || *min_values < 0
                    || *max_values < 1
                    || min_values > max_values
                    || *max_values as usize > options.len()
                    || options.iter().any(|option| {
                        option.label.is_empty()
                            || option.label.len() > 100
                            || option.value.is_empty()
                            || option.value.len() > 100
                            || option
                                .description
                                .as_ref()
                                .is_some_and(|value| value.len() > 100)
                            || option.emoji.as_ref().is_some_and(|value| value.len() > 64)
                    })
                    || !custom_ids.insert(custom_id.clone())
                {
                    return Err("Invalid interaction select menu".into());
                }
            }
        }
    }
    Ok(())
}
use super::moderation::ModerationError;
use super::permissions::{
    DEFAULT_ADMIN, DEFAULT_EVERYONE, DEFAULT_MODERATOR, Permissions, ServerRole,
};
use super::rate_limiter::RateLimiter;
use super::server::ServerState;
use super::user_session::{Protocol, UserSession};
use super::validation;

/// The default server ID used as a fallback for IRC clients
/// that don't specify a server. No server with this ID is pre-created;
/// IRC bare-channel operations will fail unless one is created by a user.
pub const DEFAULT_SERVER_ID: &str = "default";

fn moderation_wire(error: ModerationError) -> String {
    error.wire_message()
}

fn moderation_dependency() -> String {
    moderation_wire(ModerationError::DependencyUnavailable)
}

fn moderation_unavailable() -> String {
    moderation_wire(ModerationError::Unavailable)
}

fn moderation_unauthenticated() -> String {
    moderation_wire(ModerationError::Unauthenticated)
}

/// Parameters for updating notification settings (avoids too-many-arguments).
pub struct UpdateNotificationSettingsParams<'a> {
    pub server_id: &'a str,
    pub channel_id: Option<&'a str>,
    pub level: &'a str,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
    pub muted: bool,
    pub mute_until: Option<&'a str>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SearchQueryPlan {
    text: Option<String>,
    channel: Option<String>,
    sender: Option<String>,
    has_attachment: bool,
    has_link: bool,
    before: Option<String>,
    after: Option<String>,
    after_inclusive: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SearchContinuationClaims {
    exp: i64,
    credential_id: String,
    fingerprint: String,
    authorization_version: i64,
    before_created_at: String,
    before_message_id: String,
    position: i64,
}

#[derive(Debug)]
pub struct SearchResultsPage {
    pub results: Vec<super::events::SearchResultMessage>,
    pub total_count: i64,
    pub offset: i64,
    pub next_continuation: Option<String>,
    pub restarted: bool,
    pub stamp: super::authorization::AuthorizationStamp,
}

pub struct SearchMessagesRequest<'a> {
    pub server_id: &'a str,
    pub query: &'a str,
    pub channel_name: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
    pub continuation: Option<&'a str>,
}

#[derive(sqlx::FromRow)]
struct PresenceProjectionRow {
    user_id: String,
    nickname: String,
    avatar_url: Option<String>,
    requested_status: Option<String>,
    custom_status: Option<String>,
    status_emoji: Option<String>,
}

async fn server_member_display_identity(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Option<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT COALESCE(NULLIF(sm.nickname,''),u.username), \
                COALESCE(sm.avatar_url,u.avatar_url) \
         FROM server_members sm JOIN users u ON u.id=sm.user_id \
         WHERE sm.server_id=? AND sm.user_id=? AND NOT EXISTS( \
             SELECT 1 FROM bans b WHERE b.server_id=sm.server_id AND b.user_id=sm.user_id \
         )",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("INVALID_INPUT: {0}")]
    InvalidInput(String),
    #[error("INVALID_CONTINUATION: invalid search continuation")]
    InvalidContinuation,
    #[error("DEPENDENCY_UNAVAILABLE: search dependency unavailable")]
    DependencyUnavailable(#[source] super::authorization::AuthorizationError),
    #[error("RESOURCE_UNAVAILABLE: resource unavailable")]
    ResourceUnavailable,
}

impl SearchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::InvalidContinuation => "INVALID_CONTINUATION",
            Self::DependencyUnavailable(_) => "DEPENDENCY_UNAVAILABLE",
            Self::ResourceUnavailable => "RESOURCE_UNAVAILABLE",
        }
    }

    fn from_authorization(error: super::authorization::AuthorizationError) -> Self {
        match error {
            super::authorization::AuthorizationError::Unavailable => Self::ResourceUnavailable,
            other => Self::DependencyUnavailable(other),
        }
    }
}

fn parse_search_query(query: &str) -> Result<SearchQueryPlan, String> {
    if query.len() > 1_024 || query.chars().any(char::is_control) {
        return Err("invalid search query".into());
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in query.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quoted || escaped {
        return Err("invalid quoted search term".into());
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut plan = SearchQueryPlan::default();
    let mut text = Vec::new();
    for token in tokens {
        let Some((operator, value)) = token.split_once(':') else {
            text.push(token);
            continue;
        };
        match operator.to_ascii_lowercase().as_str() {
            "from" if !value.is_empty() && plan.sender.is_none() => {
                plan.sender = Some(value.to_owned())
            }
            "in" if !value.is_empty() && plan.channel.is_none() => {
                plan.channel = Some(value.to_owned())
            }
            "has" if value.eq_ignore_ascii_case("attachment") => plan.has_attachment = true,
            "has" if value.eq_ignore_ascii_case("link") => plan.has_link = true,
            "before" if plan.before.is_none() => {
                plan.before = Some(normalize_search_timestamp(value, false)?.0)
            }
            "after" if plan.after.is_none() => {
                let (boundary, date_only) = normalize_search_timestamp(value, true)?;
                plan.after = Some(boundary);
                plan.after_inclusive = date_only;
            }
            "from" | "in" | "has" | "before" | "after" => {
                return Err(format!("invalid {operator}: search filter"));
            }
            _ => text.push(token),
        }
    }
    if !text.is_empty() {
        plan.text = Some(text.join(" "));
    }
    if plan.text.is_none()
        && plan.sender.is_none()
        && plan.channel.is_none()
        && !plan.has_attachment
        && !plan.has_link
        && plan.before.is_none()
        && plan.after.is_none()
    {
        return Err("search query is empty".into());
    }
    Ok(plan)
}

fn search_fingerprint(server_id: &str, query: &str, channel_id: Option<&str>) -> String {
    let mut digest = Sha256::new();
    digest.update(server_id.as_bytes());
    digest.update([0]);
    digest.update(query.trim().as_bytes());
    digest.update([0]);
    digest.update(channel_id.unwrap_or_default().as_bytes());
    hex::encode(digest.finalize())
}

fn validate_slash_command_options(options: &[SlashCommandOption]) -> Result<(), String> {
    if options.len() > 25 {
        return Err("A command may define at most 25 options".into());
    }
    let mut names = std::collections::HashSet::new();
    let mut saw_optional = false;
    for option in options {
        if option.name.is_empty()
            || option.name.len() > 32
            || !option.name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
            || !names.insert(option.name.as_str())
        {
            return Err("Command option names must be unique lowercase identifiers".into());
        }
        if option.description.is_empty()
            || option.description.len() > 100
            || option.description.chars().any(char::is_control)
        {
            return Err("Command option descriptions must be 1-100 printable characters".into());
        }
        if !matches!(
            option.option_type.as_str(),
            "string" | "integer" | "boolean" | "user" | "channel" | "role"
        ) {
            return Err("Unsupported command option type".into());
        }
        if saw_optional && option.required {
            return Err("Required command options must precede optional options".into());
        }
        saw_optional |= !option.required;
        if let Some(choices) = &option.choices {
            if option.option_type != "string" && option.option_type != "integer" {
                return Err("Only string and integer options may define choices".into());
            }
            if choices.is_empty() || choices.len() > 25 {
                return Err("Command option choices must contain 1-25 entries".into());
            }
            let mut choice_values = std::collections::HashSet::new();
            for choice in choices {
                if choice.name.is_empty()
                    || choice.name.len() > 100
                    || choice.value.is_empty()
                    || choice.value.len() > 100
                    || !choice_values.insert(choice.value.as_str())
                {
                    return Err("Command option choices must have unique bounded values".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_slash_command_arguments(
    options: &[SlashCommandOption],
    value: &serde_json::Value,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("Command arguments must be a JSON object")?;
    if object
        .keys()
        .any(|name| !options.iter().any(|option| option.name == *name))
    {
        return Err("Command arguments contain an unknown option".into());
    }
    for option in options {
        let Some(argument) = object.get(&option.name) else {
            if option.required {
                return Err(format!("Missing required command option: {}", option.name));
            }
            continue;
        };
        let type_matches = match option.option_type.as_str() {
            "string" | "user" | "channel" | "role" => argument.is_string(),
            "integer" => argument.as_i64().is_some(),
            "boolean" => argument.is_boolean(),
            _ => false,
        };
        if !type_matches {
            return Err(format!("Invalid value for command option: {}", option.name));
        }
        if let Some(choices) = &option.choices {
            let candidate = argument
                .as_str()
                .map(str::to_owned)
                .or_else(|| argument.as_i64().map(|number| number.to_string()))
                .ok_or_else(|| format!("Invalid value for command option: {}", option.name))?;
            if !choices.iter().any(|choice| choice.value == candidate) {
                return Err(format!(
                    "Invalid choice for command option: {}",
                    option.name
                ));
            }
        }
    }
    Ok(())
}

fn normalize_search_timestamp(value: &str, next_day: bool) -> Result<(String, bool), String> {
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok((timestamp.with_timezone(&Utc).to_rfc3339(), false));
    }
    let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "invalid search timestamp".to_string())?;
    let boundary = if next_day {
        date.succ_opt()
            .ok_or_else(|| "invalid search timestamp".to_string())?
    } else {
        date
    };
    Ok((
        boundary
            .and_time(chrono::NaiveTime::MIN)
            .and_utc()
            .to_rfc3339(),
        true,
    ))
}

pub enum Synchronization {
    Snapshot(super::replay::SyncSnapshot),
    Replay(super::replay::ReplayBatch),
}

/// The central hub that manages all chat state. Protocol-agnostic —
/// both IRC and WebSocket adapters call into this.
pub struct ChatEngine {
    /// All currently connected sessions, keyed by session ID.
    sessions: DashMap<ConnectionId, Arc<UserSession>>,
    user_connections: DashMap<String, std::collections::HashSet<ConnectionId>>,
    /// All servers (guilds), keyed by server ID.
    servers: DashMap<String, ServerState>,
    /// All channels, keyed by channel UUID.
    channels: DashMap<String, ChannelState>,
    /// Index: (server_id, channel_name) -> channel_id for name-based lookups.
    channel_name_index: DashMap<(String, String), String>,
    server_alias_index: DashMap<String, String>,
    server_aliases: DashMap<String, String>,
    /// Reverse lookup: nickname -> session ID (for DMs and WHOIS).
    nick_to_session: DashMap<String, ConnectionId>,
    /// Durable authenticated principal bound to each production connection.
    authenticated_actors: DashMap<ConnectionId, crate::auth::authority::Actor>,
    credential_connections:
        DashMap<crate::auth::authority::CredentialId, std::collections::HashSet<ConnectionId>>,
    /// Optional database pool. When present, messages and channels are persisted.
    db: Option<SqlitePool>,
    auth: OnceLock<crate::auth::authority::AuthService>,
    messaging: OnceLock<super::messaging::MessagingService>,
    replay: OnceLock<super::replay::ReplayService>,
    search_token_secret: String,
    integration_vault: OnceLock<Arc<crate::secrets::SecretVault>>,
    write_admission: Option<super::write_admission::WriteAdmission>,
    /// Per-user message rate limiter (burst of 10, refill 1 per second).
    message_limiter: RateLimiter,
    /// Maximum message content length (configurable, default 4000).
    max_message_length: usize,
    /// Maximum file upload size in megabytes (configurable, default 100).
    max_file_size_mb: u64,
    /// In-memory slow mode tracker: (user_id, channel_id) -> last message Instant.
    /// Prevents concurrent requests from bypassing the DB-based cooldown check.
    slowmode_last_sent: DashMap<(String, String), Instant>,
}

impl ChatEngine {
    pub(crate) async fn begin_admitted_write(
        &self,
    ) -> Result<
        (
            tokio::sync::OwnedSemaphorePermit,
            sqlx::Transaction<'static, sqlx::Sqlite>,
        ),
        String,
    > {
        self.write_admission
            .as_ref()
            .ok_or_else(|| "DEPENDENCY_UNAVAILABLE: write dependency unavailable".to_string())?
            .begin()
            .await
            .map_err(|_| "DEPENDENCY_UNAVAILABLE: write dependency unavailable".to_string())
    }
    pub fn new(
        db: SqlitePool,
        auth: crate::auth::authority::AuthService,
        replay_secret: &str,
        max_message_length: usize,
        max_file_size_mb: u64,
    ) -> Self {
        let write_admission = super::write_admission::WriteAdmission::new(db.clone());
        let messaging = super::messaging::MessagingService::new_with_write_admission(
            db.clone(),
            auth.clone(),
            max_message_length,
            write_admission.clone(),
        );
        let replay = super::replay::ReplayService::new_with_write_admission(
            db.clone(),
            auth.clone(),
            replay_secret,
            write_admission.clone(),
        );
        Self {
            sessions: DashMap::new(),
            user_connections: DashMap::new(),
            servers: DashMap::new(),
            channels: DashMap::new(),
            channel_name_index: DashMap::new(),
            server_alias_index: DashMap::new(),
            server_aliases: DashMap::new(),
            nick_to_session: DashMap::new(),
            authenticated_actors: DashMap::new(),
            credential_connections: DashMap::new(),
            db: Some(db),
            auth: OnceLock::from(auth),
            messaging: OnceLock::from(messaging),
            replay: OnceLock::from(replay),
            search_token_secret: replay_secret.to_owned(),
            integration_vault: OnceLock::new(),
            write_admission: Some(write_admission),
            message_limiter: RateLimiter::new(10, 1.0),
            max_message_length,
            max_file_size_mb,
            slowmode_last_sent: DashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_harness(max_message_length: usize, max_file_size_mb: u64) -> Self {
        Self {
            sessions: DashMap::new(),
            user_connections: DashMap::new(),
            servers: DashMap::new(),
            channels: DashMap::new(),
            channel_name_index: DashMap::new(),
            server_alias_index: DashMap::new(),
            server_aliases: DashMap::new(),
            nick_to_session: DashMap::new(),
            authenticated_actors: DashMap::new(),
            credential_connections: DashMap::new(),
            db: None,
            auth: OnceLock::new(),
            messaging: OnceLock::new(),
            replay: OnceLock::new(),
            search_token_secret: "test-search-token-secret".into(),
            integration_vault: OnceLock::new(),
            write_admission: None,
            message_limiter: RateLimiter::new(10, 1.0),
            max_message_length,
            max_file_size_mb,
            slowmode_last_sent: DashMap::new(),
        }
    }

    pub fn replay_service(&self) -> &super::replay::ReplayService {
        self.replay
            .get()
            .expect("production constructor installs replay service")
    }

    /// Poll the transactional outbox. Wakeups reduce latency; polling guarantees
    /// recovery after process restart or a missed bounded hint.
    pub async fn run_delivery_dispatcher(
        self: Arc<Self>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        let messaging = self
            .messaging
            .get()
            .ok_or_else(|| "messaging service unavailable".to_string())?;
        let mut wakeups = messaging.subscribe_wakeups();
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(500));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut maintenance_ticks = 0_u32;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = poll.tick() => {},
                _ = wakeups.recv() => {},
            }
            self.process_moderation_cleanup_batch().await?;
            loop {
                if shutdown.is_cancelled() || self.dispatch_outbox_batch().await? < 100 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            maintenance_ticks = maintenance_ticks.wrapping_add(1);
            if maintenance_ticks.is_multiple_of(120) {
                self.prune_delivery_retention().await?;
                self.archive_due_threads().await?;
            }
        }
    }

    async fn archive_due_threads(&self) -> Result<(), String> {
        let writes = self
            .write_admission
            .as_ref()
            .ok_or_else(|| "thread write admission unavailable".to_string())?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        let thread_versions =
            crate::db::queries::threads::archive_due_threads(&mut transaction, 100)
                .await
                .map_err(|error| error.to_string())?;
        for (thread_id, version) in &thread_versions {
            Self::insert_thread_state_event_in(
                &mut transaction,
                thread_id,
                *version,
                true,
                Some("inactivity"),
                "system",
            )
            .await?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        for (thread_id, _) in thread_versions {
            self.project_thread_state(&thread_id).await?;
        }
        Ok(())
    }

    /// Advances at most one ban-cleanup job and at most one hundred messages.
    /// The job row and every canonical tombstone/event are committed together,
    /// so restart resumes from the remaining undeleted rows.
    async fn process_moderation_cleanup_batch(&self) -> Result<usize, String> {
        let writes = self
            .write_admission
            .as_ref()
            .ok_or_else(|| "moderation cleanup write admission unavailable".to_string())?;
        let (_permit, mut transaction) =
            writes.begin().await.map_err(|_| "resource unavailable")?;
        let job = sqlx::query(
            "SELECT id,server_id,user_id,actor_id,cutoff_at \
             FROM moderation_cleanup_jobs WHERE state='pending' \
             ORDER BY created_at,id LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        let Some(job) = job else {
            transaction
                .commit()
                .await
                .map_err(|_| "resource unavailable")?;
            return Ok(0);
        };
        let job_id: String = job.get(0);
        let server_id: String = job.get(1);
        let user_id: String = job.get(2);
        let actor_id: String = job.get(3);
        let cutoff_at: String = job.get(4);
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| "resource unavailable")?;
        let messages = sqlx::query(
            "SELECT m.id FROM messages m \
             JOIN channels c ON c.id=m.channel_id AND c.server_id=m.server_id \
             JOIN moderation_cleanup_scopes s ON s.job_id=? \
                AND s.conversation_id=m.conversation_id \
                AND m.conversation_sequence<=s.through_sequence \
             WHERE m.server_id=? AND m.sender_id=? AND m.deleted_at IS NULL \
               AND julianday(m.created_at)>=julianday(?) \
             ORDER BY m.created_at,m.id LIMIT 100",
        )
        .bind(&job_id)
        .bind(&server_id)
        .bind(&user_id)
        .bind(&cutoff_at)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        for message in &messages {
            let message_id: String = message.get(0);
            super::messaging::tombstone_moderated_message_in(
                &mut transaction,
                &generation,
                &message_id,
                &actor_id,
            )
            .await
            .map_err(|_| "resource unavailable")?
            .ok_or_else(|| "resource unavailable".to_string())?;
        }
        let remaining: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages m \
             JOIN channels c ON c.id=m.channel_id AND c.server_id=m.server_id \
             JOIN moderation_cleanup_scopes s ON s.job_id=? \
                AND s.conversation_id=m.conversation_id \
                AND m.conversation_sequence<=s.through_sequence \
             WHERE m.server_id=? AND m.sender_id=? AND m.deleted_at IS NULL \
               AND julianday(m.created_at)>=julianday(?))",
        )
        .bind(&job_id)
        .bind(&server_id)
        .bind(&user_id)
        .bind(&cutoff_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        sqlx::query(
            "UPDATE moderation_cleanup_jobs SET deleted_count=deleted_count+?, \
             state=?,updated_at=datetime('now') WHERE id=?",
        )
        .bind(messages.len() as i64)
        .bind(if remaining { "pending" } else { "completed" })
        .bind(&job_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable")?;
        Ok(messages.len())
    }

    async fn insert_thread_state_event_in(
        connection: &mut sqlx::SqliteConnection,
        thread_id: &str,
        version: i64,
        archived: bool,
        reason: Option<&str>,
        actor_id: &str,
    ) -> Result<(), String> {
        let scope: Option<(String, i64)> = sqlx::query_as(
            "SELECT cv.id,c.authorization_version FROM channels c \
             JOIN conversations cv ON cv.channel_id=c.id WHERE c.id=?",
        )
        .bind(thread_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        let (conversation_id, authorization_version) =
            scope.ok_or_else(|| "thread conversation unavailable".to_string())?;
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
             VALUES('thread_state',?,?,datetime('now')) \
             ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
                version=excluded.version,updated_at=excluded.updated_at",
        )
        .bind(thread_id)
        .bind(version)
        .execute(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *connection)
                .await
                .map_err(|error| error.to_string())?;
        let event_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'thread_state_changed','thread_state',?,?,?,?,?) \
             RETURNING event_sequence",
        )
        .bind(generation)
        .bind(conversation_id)
        .bind(thread_id)
        .bind(version)
        .bind(authorization_version)
        .bind(actor_id)
        .bind(serde_json::json!({"archived": archived, "reason": reason}).to_string())
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
            .bind(event_sequence)
            .execute(connection)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn project_thread_state(&self, thread_id: &str) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let persisted: Option<(i64, i64)> =
            sqlx::query_as("SELECT archived,thread_state_version FROM channels WHERE id=?")
                .bind(thread_id)
                .fetch_optional(pool)
                .await
                .map_err(|error| error.to_string())?;
        let Some((archived, state_version)) = persisted else {
            return Ok(());
        };
        self.apply_thread_state_projection(thread_id, archived != 0, state_version);
        Ok(())
    }

    fn apply_thread_state_projection(&self, thread_id: &str, archived: bool, state_version: i64) {
        let Some(mut channel) = self.channels.get_mut(thread_id) else {
            return;
        };
        if state_version < channel.thread_state_version {
            return;
        }
        channel.archived = archived;
        channel.thread_state_version = state_version;
        let event = ChatEvent::ThreadUpdate {
            server_id: channel.server_id.clone(),
            thread: ThreadInfo {
                id: channel.id.clone(),
                name: channel.name.clone(),
                channel_type: channel.channel_type.clone(),
                parent_message_id: channel.thread_parent_message_id.clone(),
                creator_user_id: channel.thread_creator_user_id.clone(),
                archived,
                state_version,
                tags_version: channel.thread_tags_version,
                tag_ids: channel.thread_tag_ids.clone(),
                auto_archive_minutes: channel.auto_archive_minutes,
                message_count: 0,
                created_at: channel.created_at.to_rfc3339(),
            },
        };
        drop(channel);
        self.broadcast_to_channel(thread_id, &event, None);
    }

    async fn prune_delivery_retention(&self) -> Result<usize, String> {
        let pool = self
            .db
            .as_ref()
            .ok_or_else(|| "delivery database unavailable".to_string())?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| error.to_string())?;
        sqlx::query(
            "DELETE FROM command_receipts WHERE rowid IN ( \
                 SELECT cr.rowid FROM command_receipts cr \
                 JOIN operation_generations g ON g.generation=cr.operation_generation \
                 WHERE g.expires_at<=unixepoch() \
                   AND (cr.canonical_message_id IS NULL OR NOT EXISTS( \
                       SELECT 1 FROM messages m WHERE m.id=cr.canonical_message_id \
                   )) \
                 ORDER BY cr.rowid LIMIT 500 \
             )",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        let candidates: Vec<i64> = sqlx::query_scalar(
            "SELECT e.event_sequence FROM event_log e \
             JOIN delivery_outbox o ON o.event_sequence=e.event_sequence \
             JOIN event_retention_state r ON r.singleton=1 \
             WHERE o.completed_at IS NOT NULL \
               AND e.event_sequence<=r.dispatcher_high_water \
               AND e.created_at<datetime('now','-' || r.retention_seconds || ' seconds') \
             ORDER BY e.event_sequence LIMIT 500",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        for event_sequence in &candidates {
            sqlx::query(
                "DELETE FROM delivery_outbox WHERE event_sequence=? AND completed_at IS NOT NULL",
            )
            .bind(event_sequence)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM event_log WHERE event_sequence=?")
                .bind(event_sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
        }
        sqlx::query(
            "UPDATE event_retention_state SET retained_from_sequence=COALESCE( \
                 (SELECT MIN(event_sequence) FROM event_log), \
                 (SELECT seq+1 FROM sqlite_sequence WHERE name='event_log'),0 \
             ),updated_at=datetime('now') WHERE singleton=1",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        Ok(candidates.len())
    }

    async fn dispatch_outbox_batch(&self) -> Result<usize, String> {
        let pool = self
            .db
            .as_ref()
            .ok_or_else(|| "delivery database unavailable".to_string())?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| error.to_string())?;
        let mut event_sequences: Vec<i64> = sqlx::query_scalar(
            "UPDATE delivery_outbox SET attempts=attempts+1, \
                    claimed_until=datetime('now','+30 seconds'),last_error=NULL \
             WHERE event_sequence IN ( \
                 SELECT event_sequence FROM delivery_outbox \
                 WHERE completed_at IS NULL AND available_at<=datetime('now') \
                   AND (claimed_until IS NULL OR claimed_until<=datetime('now')) \
                 ORDER BY event_sequence LIMIT 100 \
             ) RETURNING event_sequence",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        event_sequences.sort_unstable();

        for event_sequence in &event_sequences {
            let target = sqlx::query(
                "SELECT c.kind,c.channel_id,c.id FROM event_log e \
                 JOIN conversations c ON c.id=e.conversation_id WHERE e.event_sequence=?",
            )
            .bind(event_sequence)
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?;
            let mut session_ids = std::collections::HashSet::new();
            if let Some(target) = target {
                let kind: String = target.get(0);
                if kind == "channel" {
                    if let Some(channel_id) = target.get::<Option<String>, _>(1)
                        && let Some(channel) = self.channels.get(&channel_id)
                    {
                        session_ids.extend(channel.members.iter().copied());
                    }
                } else {
                    let conversation_id: String = target.get(2);
                    let participants: Vec<String> = sqlx::query_scalar(
                        "SELECT user_id FROM conversation_participants \
                         WHERE conversation_id=? AND left_at IS NULL",
                    )
                    .bind(conversation_id)
                    .fetch_all(pool)
                    .await
                    .map_err(|error| error.to_string())?;
                    for participant in participants {
                        if let Some(connections) = self.user_connections.get(&participant) {
                            session_ids.extend(connections.iter().copied());
                        }
                    }
                }
            }
            let mut failed = None;
            for session_id in session_ids {
                let Some(actor) = self
                    .authenticated_actors
                    .get(&session_id)
                    .map(|entry| entry.clone())
                else {
                    continue;
                };
                match self
                    .replay_service()
                    .project_event(&actor, *event_sequence)
                    .await
                {
                    Ok(Some((conversation_id, event))) => {
                        if let Some(session) = self.sessions.get(&session_id) {
                            session.send_guarded(
                                ChatEvent::DurableEvent {
                                    event: Box::new(event),
                                },
                                Some(super::user_session::DeliveryGuard::Conversations(vec![
                                    conversation_id.into_inner(),
                                ])),
                            );
                        }
                    }
                    Ok(None)
                    | Err(super::replay::ReplayError::ResyncRequired(_))
                    | Err(super::replay::ReplayError::Unavailable) => {}
                    Err(error) => {
                        failed = Some(error.to_string());
                        break;
                    }
                }
            }
            if let Some(error) = failed {
                sqlx::query(
                    "UPDATE delivery_outbox SET claimed_until=NULL,last_error=?, \
                            available_at=datetime('now','+1 second') WHERE event_sequence=?",
                )
                .bind(error)
                .bind(event_sequence)
                .execute(pool)
                .await
                .map_err(|error| error.to_string())?;
            } else {
                let mut transaction = pool
                    .begin_with("BEGIN IMMEDIATE")
                    .await
                    .map_err(|error| error.to_string())?;
                sqlx::query(
                    "UPDATE delivery_outbox SET completed_at=datetime('now'),claimed_until=NULL \
                     WHERE event_sequence=?",
                )
                .bind(event_sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
                sqlx::query(
                    "UPDATE event_retention_state SET dispatcher_high_water=( \
                         SELECT COALESCE(MIN(event_sequence)-1, \
                             (SELECT COALESCE(MAX(event_sequence),0) FROM event_log)) \
                         FROM delivery_outbox WHERE completed_at IS NULL \
                     ),updated_at=datetime('now') WHERE singleton=1",
                )
                .execute(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
                transaction
                    .commit()
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(event_sequences.len())
    }

    pub async fn synchronize(
        &self,
        session_id: ConnectionId,
        subscriptions: &[String],
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Synchronization, super::replay::ReplayError> {
        let actor = self
            .authenticated_actors
            .get(&session_id)
            .map(|entry| entry.clone())
            .ok_or(super::replay::ReplayError::Unavailable)?;
        if let Some(cursor) = cursor {
            self.replay
                .get()
                .ok_or(super::replay::ReplayError::Unavailable)?
                .replay(&actor, subscriptions, cursor, limit)
                .await
                .map(Synchronization::Replay)
        } else {
            self.replay
                .get()
                .ok_or(super::replay::ReplayError::Unavailable)?
                .snapshot_with_limit(&actor, subscriptions, limit)
                .await
                .map(Synchronization::Snapshot)
        }
    }

    pub async fn conversation_id_for_channel(
        &self,
        server_id: &str,
        channel: &str,
    ) -> Result<String, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        sqlx::query_scalar(
            "SELECT cv.id FROM conversations cv JOIN channels c ON c.id=cv.channel_id \
             WHERE cv.kind='channel' AND c.server_id=? AND c.name=?",
        )
        .bind(server_id)
        .bind(normalize_channel_name(channel))
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("DB error: {error}"))?
        .ok_or_else(|| "resource unavailable".into())
    }

    pub fn bind_authenticated_actor(
        &self,
        session_id: ConnectionId,
        actor: crate::auth::authority::Actor,
    ) -> Result<(), String> {
        let session = self.sessions.get(&session_id).ok_or("Session not found")?;
        if session.user_id.as_deref() != Some(actor.user_id().as_str()) {
            return Err("authenticated actor does not match connection identity".into());
        }
        self.credential_connections
            .entry(actor.credential_id().clone())
            .or_default()
            .insert(session_id);
        self.authenticated_actors.insert(session_id, actor);
        Ok(())
    }

    pub fn get_authenticated_actor(
        &self,
        session_id: ConnectionId,
    ) -> Option<crate::auth::authority::Actor> {
        self.authenticated_actors
            .get(&session_id)
            .map(|actor| actor.clone())
    }

    /// Get the configured maximum message length.
    pub fn max_message_length(&self) -> usize {
        self.max_message_length
    }

    /// Get the configured maximum file upload size in megabytes.
    pub fn max_file_size_mb(&self) -> u64 {
        self.max_file_size_mb
    }

    /// Remove stale rate-limiter buckets that haven't been used recently.
    pub fn cleanup_rate_limiter(&self) {
        self.message_limiter
            .cleanup(std::time::Duration::from_secs(600));
    }

    /// Remove stale slow mode cache entries older than the given duration.
    pub fn cleanup_slowmode_cache(&self) {
        let cutoff = std::time::Duration::from_secs(600);
        self.slowmode_last_sent
            .retain(|_, instant| instant.elapsed() < cutoff);
    }

    // ── Startup loading ─────────────────────────────────────────────

    /// Load servers from the database into memory on startup.
    pub async fn load_servers_from_db(&self) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Ok(());
        };

        let rows = crate::db::queries::servers::list_all_servers(pool)
            .await
            .map_err(|e| format!("Failed to load servers: {e}"))?;

        for row in rows {
            let mut state =
                ServerState::new(row.id.clone(), row.name, row.owner_id.clone(), row.icon_url);

            let members = crate::db::queries::servers::get_server_members(pool, &row.id)
                .await
                .map_err(|e| format!("Failed to load server members: {e}"))?;
            for m in members {
                state.member_user_ids.insert(m.user_id);
            }

            // Bootstrap default roles for servers that don't have any
            if !crate::db::queries::roles::server_has_roles(pool, &row.id)
                .await
                .unwrap_or(true)
            {
                info!(server_id = %row.id, "bootstrapping default roles for existing server");
                let default_roles = [
                    ("@everyone", None, 0, DEFAULT_EVERYONE.bits() as i64, true),
                    ("Moderator", None, 1, DEFAULT_MODERATOR.bits() as i64, false),
                    ("Admin", None, 2, DEFAULT_ADMIN.bits() as i64, false),
                    ("Owner", None, 3, Permissions::all().bits() as i64, false),
                ];
                let mut owner_role_id = None;
                for (role_name, color, position, perms, is_default) in &default_roles {
                    let role_id = Uuid::new_v4().to_string();
                    let params = crate::db::queries::roles::CreateRoleParams {
                        id: &role_id,
                        server_id: &row.id,
                        name: role_name,
                        color: *color,
                        icon_url: None,
                        position: *position,
                        permissions: *perms,
                        is_default: *is_default,
                    };
                    if let Err(e) = crate::db::queries::roles::create_role(pool, &params).await {
                        warn!(error = %e, role = role_name, "failed to create default role on load");
                    }
                    if *role_name == "Owner" {
                        owner_role_id = Some(role_id);
                    }
                }
                // Assign Owner role to the server owner
                if let Some(role_id) = owner_role_id
                    && let Err(e) = crate::db::queries::roles::assign_role(
                        pool,
                        &row.id,
                        &row.owner_id,
                        &role_id,
                    )
                    .await
                {
                    warn!(error = %e, "failed to assign Owner role on load");
                }
            }

            self.servers.insert(row.id, state);
        }
        let aliases = sqlx::query_as::<_, (String, String)>(
            "SELECT alias,server_id FROM server_aliases ORDER BY is_canonical DESC,created_at",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to load server aliases: {e}"))?;
        for (alias, server_id) in aliases {
            self.server_alias_index
                .insert(alias.to_lowercase(), server_id.clone());
            self.server_aliases.entry(server_id).or_insert(alias);
        }

        info!(count = self.servers.len(), "loaded servers from database");
        Ok(())
    }

    /// Load channels from the database into memory on startup.
    pub async fn load_channels_from_db(&self) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Ok(());
        };

        // Collect server IDs first to avoid holding a read lock on self.servers
        // while later acquiring a write lock via get_mut (DashMap deadlock).
        let server_ids: Vec<String> = self.servers.iter().map(|s| s.id.clone()).collect();

        for server_id in &server_ids {
            let rows = crate::db::queries::channels::list_channels(pool, server_id)
                .await
                .map_err(|e| format!("Failed to load channels: {e}"))?;

            for row in rows {
                let mut ch =
                    ChannelState::new(row.id.clone(), row.server_id.clone(), row.name.clone());
                ch.topic = row.topic;
                ch.topic_set_by = row.topic_set_by;
                ch.category_id = row.category_id;
                ch.position = row.position;
                ch.is_private = row.is_private != 0;
                ch.channel_type = row.channel_type;
                ch.thread_parent_message_id = row.thread_parent_message_id;
                ch.thread_creator_user_id = row.thread_creator_user_id;
                ch.auto_archive_minutes = row.thread_auto_archive_minutes;
                ch.archived = row.archived != 0;
                ch.thread_state_version = row.thread_state_version;
                ch.thread_tags_version = row.thread_tags_version;
                if matches!(ch.channel_type.as_str(), "public_thread" | "private_thread") {
                    ch.thread_tag_ids = sqlx::query_scalar(
                        "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
                    )
                    .bind(&row.id)
                    .fetch_all(pool)
                    .await
                    .map_err(|error| format!("Failed to load thread tags: {error}"))?;
                }
                ch.slowmode_seconds = row.slowmode_seconds;
                ch.is_nsfw = row.is_nsfw != 0;

                self.channel_name_index
                    .insert((row.server_id.clone(), row.name), row.id.clone());

                if let Some(mut srv) = self.servers.get_mut(&row.server_id) {
                    srv.channel_ids.insert(row.id.clone());
                }

                self.channels.insert(row.id, ch);
            }
        }

        info!(count = self.channels.len(), "loaded channels from database");
        Ok(())
    }

    // ── Session management ──────────────────────────────────────────

    /// Register a new session. Returns the session ID and an event receiver.
    pub fn connect(
        &self,
        user_id: Option<String>,
        nickname: String,
        protocol: Protocol,
        avatar_url: Option<String>,
    ) -> Result<(ConnectionId, mpsc::Receiver<ChatEvent>), String> {
        match protocol {
            Protocol::Irc => validation::validate_nickname(&nickname)?,
            Protocol::WebSocket => validation::validate_display_name(&nickname)?,
        }
        let nickname_key = crate::auth::authority::rfc1459_casefold(&nickname);

        // A user may connect from several clients with the same stable nickname.
        // The nickname remains exclusive across different user identities.
        if let Some(old_session_id) = self
            .nick_to_session
            .get(&nickname_key)
            .map(|entry| *entry.value())
        {
            let same_user = self
                .sessions
                .get(&old_session_id)
                .is_some_and(|session| user_id.is_some() && session.user_id == user_id);
            if !same_user {
                return Err(format!("Nickname already in use: {nickname}"));
            }
        }

        let session_id = ConnectionId::new();
        let (tx, rx) = mpsc::channel(crate::engine::user_session::MAX_OUTBOUND_QUEUE);

        let session = Arc::new(UserSession::new(
            session_id,
            user_id,
            nickname.clone(),
            protocol,
            tx,
            avatar_url,
        ));

        // Capture user_id before moving session into the map
        let session_user_id = session.user_id.clone();

        self.sessions.insert(session_id, session);
        self.nick_to_session.insert(nickname_key, session_id);
        if let Some(user_id) = &session_user_id {
            self.user_connections
                .entry(user_id.clone())
                .or_default()
                .insert(session_id);
        }

        // Update presence to online
        if let (Some(uid), Some(pool)) = (&session_user_id, &self.db) {
            let pool = pool.clone();
            let uid = uid.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::db::queries::presence::set_connected(&pool, &uid).await {
                    tracing::warn!(error = %e, "failed to update presence to online");
                }
            });
        }

        info!(%session_id, %nickname, ?protocol, "session connected");

        Ok((session_id, rx))
    }

    /// Disconnect a session and clean up all state.
    pub fn disconnect(&self, session_id: ConnectionId) {
        if let Some((_, actor)) = self.authenticated_actors.remove(&session_id)
            && let Some(mut connections) =
                self.credential_connections.get_mut(actor.credential_id())
        {
            connections.remove(&session_id);
            if connections.is_empty() {
                let credential_id = actor.credential_id().clone();
                drop(connections);
                self.credential_connections.remove(&credential_id);
            }
        }
        let Some((_, session)) = self.sessions.remove(&session_id) else {
            return;
        };

        let nickname = session.nickname.clone();
        if let Some(user_id) = &session.user_id
            && let Some(mut connections) = self.user_connections.get_mut(user_id)
        {
            connections.remove(&session_id);
            if connections.is_empty() {
                drop(connections);
                self.user_connections.remove(user_id);
            }
        }
        let nickname_key = crate::auth::authority::rfc1459_casefold(&nickname);
        if self
            .nick_to_session
            .get(&nickname_key)
            .is_some_and(|indexed| *indexed == session_id)
        {
            self.nick_to_session.remove(&nickname_key);
            if let Some(replacement) = self
                .sessions
                .iter()
                .find(|candidate| candidate.nickname == nickname)
                .map(|candidate| *candidate.key())
            {
                self.nick_to_session.insert(nickname_key, replacement);
            }
        }

        // Collect channels this session was in
        let channels_to_leave: Vec<String> = self
            .channels
            .iter()
            .filter(|ch| ch.members.contains(&session_id))
            .map(|ch| ch.key().clone())
            .collect();

        for channel_id in &channels_to_leave {
            if let Some(mut channel) = self.channels.get_mut(channel_id) {
                channel.members.remove(&session_id);
            }
        }

        let has_other_user_connection = session.user_id.as_ref().is_some_and(|uid| {
            self.user_connections
                .get(uid)
                .is_some_and(|connections| !connections.is_empty())
        });
        if !has_other_user_connection {
            let quit_event = ChatEvent::Quit {
                nickname: nickname.clone(),
                reason: None,
            };
            for channel_id in &channels_to_leave {
                self.broadcast_to_channel(channel_id, &quit_event, Some(session_id));
            }
        }

        // Update presence if this was the last session for this user
        if let Some(ref uid) = session.user_id {
            let other_sessions = self
                .user_connections
                .get(uid)
                .is_some_and(|connections| !connections.is_empty());
            if !other_sessions {
                if let Some(pool) = &self.db {
                    let _ = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(crate::db::queries::presence::set_offline(pool, uid))
                    });
                }
                // Broadcast offline using each server's durable display identity.
                let server_ids: Vec<String> = self
                    .servers
                    .iter()
                    .filter(|server| server.member_user_ids.contains(uid))
                    .map(|server| server.id.clone())
                    .collect();
                for server_id in server_ids {
                    let identity = self.db.as_ref().and_then(|pool| {
                        match tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(
                                server_member_display_identity(pool, &server_id, uid),
                            )
                        }) {
                            Ok(identity) => identity,
                            Err(error) => {
                                warn!(%error, %server_id, user_id = %uid, "offline presence identity query failed");
                                None
                            }
                        }
                    });
                    if let (Some((nickname, avatar_url)), Some(server)) =
                        (identity, self.servers.get(&server_id))
                    {
                        let event = ChatEvent::PresenceUpdate {
                            server_id: server_id.clone(),
                            presence: super::events::PresenceInfo {
                                user_id: uid.clone(),
                                nickname,
                                avatar_url,
                                status: "offline".into(),
                                custom_status: None,
                                status_emoji: None,
                            },
                        };
                        let mut notified = std::collections::HashSet::new();
                        for channel_id in server.channel_ids.iter() {
                            if let Some(channel) = self.channels.get(channel_id) {
                                for &member_sid in &channel.members {
                                    if member_sid != session_id
                                        && notified.insert(member_sid)
                                        && let Some(s) = self.sessions.get(&member_sid)
                                    {
                                        let _ = s.send(event.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(%session_id, %nickname, "session disconnected");
    }

    // ── Server management ───────────────────────────────────────────

    /// Create a new server. Returns the server ID.
    pub async fn create_server(
        &self,
        name: String,
        owner_user_id: String,
        icon_url: Option<String>,
    ) -> Result<String, String> {
        validation::validate_server_name(&name)?;

        let server_id = Uuid::new_v4().to_string();
        let channel_id = Uuid::new_v4().to_string();
        let server_alias = stable_irc_alias(&name, &server_id);

        if self.db.is_some() {
            return Err("authenticated actor required".into());
        } else {
            let owned_count = self
                .servers
                .iter()
                .filter(|server| server.owner_id == owner_user_id)
                .count();
            if owned_count >= 100 {
                return Err("server limit reached".to_string());
            }
        }

        let mut state = ServerState::new(
            server_id.clone(),
            name.clone(),
            owner_user_id.clone(),
            icon_url,
        );
        state.member_user_ids.insert(owner_user_id.clone());
        self.servers.insert(server_id.clone(), state);
        self.server_alias_index
            .insert(server_alias.clone(), server_id.clone());
        self.server_aliases.insert(server_id.clone(), server_alias);

        // Mirror the committed default channel in the runtime cache.
        let channel_name = "#general".to_string();
        let ch = ChannelState::new(channel_id.clone(), server_id.clone(), channel_name.clone());
        self.channel_name_index
            .insert((server_id.clone(), channel_name), channel_id.clone());
        if let Some(mut srv) = self.servers.get_mut(&server_id) {
            srv.channel_ids.insert(channel_id.clone());
        }
        self.channels.insert(channel_id, ch);

        info!(%server_id, %name, "server created");
        Ok(server_id)
    }

    pub async fn create_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        name: String,
        icon_url: Option<String>,
    ) -> Result<String, String> {
        validation::validate_server_name(&name)?;
        let owner_user_id = actor.user_id().as_str().to_owned();
        let server_id = Uuid::new_v4().to_string();
        let channel_id = Uuid::new_v4().to_string();
        let server_alias = stable_irc_alias(&name, &server_id);
        self.organization_service()?
            .provision_server(
                actor,
                &name,
                icon_url.as_deref(),
                &referenced_server_id(&server_id)?,
                &referenced_channel_id(&channel_id)?,
                &server_alias,
            )
            .await
            .map_err(String::from)?;
        let mut state = ServerState::new(server_id.clone(), name, owner_user_id.clone(), icon_url);
        state.member_user_ids.insert(owner_user_id);
        self.servers.insert(server_id.clone(), state);
        self.server_alias_index
            .insert(server_alias.clone(), server_id.clone());
        self.server_aliases.insert(server_id.clone(), server_alias);
        let channel_name = "#general".to_string();
        let channel =
            ChannelState::new(channel_id.clone(), server_id.clone(), channel_name.clone());
        self.channel_name_index
            .insert((server_id.clone(), channel_name), channel_id.clone());
        if let Some(mut server) = self.servers.get_mut(&server_id) {
            server.channel_ids.insert(channel_id.clone());
        }
        self.channels.insert(channel_id, channel);
        Ok(server_id)
    }

    /// Delete a server.
    pub async fn delete_server(&self, server_id: &str) -> Result<(), String> {
        if let Some(pool) = &self.db {
            crate::db::queries::servers::delete_server(pool, server_id)
                .await
                .map_err(|e| format!("Failed to delete server: {e}"))?;
        }

        self.remove_server_from_cache(server_id);

        info!(%server_id, "server deleted");
        Ok(())
    }

    pub async fn admin_delete_server(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .admin_delete_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        self.remove_server_from_cache(server_id);
        Ok(())
    }

    pub async fn delete_owned_server(
        &self,
        server_id: &str,
        actor: &crate::auth::authority::Actor,
    ) -> Result<(), String> {
        self.organization_service()?
            .delete_owned_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        self.remove_server_from_cache(server_id);
        info!(%server_id, "server deleted");
        Ok(())
    }

    fn remove_server_from_cache(&self, server_id: &str) {
        if let Some(server) = self.servers.get(server_id) {
            for ch_id in &server.channel_ids {
                if let Some((_, ch)) = self.channels.remove(ch_id) {
                    self.channel_name_index
                        .remove(&(server_id.to_string(), ch.name));
                }
            }
        }
        if let Some((_, alias)) = self.server_aliases.remove(server_id) {
            self.server_alias_index.remove(&alias);
        }
        self.servers.remove(server_id);
    }

    /// Update a server's name and/or icon.
    pub async fn update_server_settings(
        &self,
        server_id: &str,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(), String> {
        // Compute new values and apply in-memory update while holding the guard,
        // then drop the guard before any .await to avoid holding the DashMap shard
        // lock across an async suspension point.
        let (new_name, new_icon) = {
            let mut server = self
                .servers
                .get_mut(server_id)
                .ok_or_else(|| format!("No such server: {server_id}"))?;

            let new_name = name.unwrap_or(&server.name).to_string();
            let new_icon = if icon_url.is_some() {
                icon_url.map(|s| s.to_string())
            } else {
                server.icon_url.clone()
            };

            server.name = new_name.clone();
            server.icon_url = new_icon.clone();

            (new_name, new_icon)
        }; // guard dropped here

        if let Some(pool) = &self.db
            && let Err(e) = crate::db::queries::servers::update_server(
                pool,
                server_id,
                &new_name,
                new_icon.as_deref(),
            )
            .await
        {
            warn!(%server_id, error = %e, "failed to persist server settings update to DB");
            return Err(format!("Failed to update server: {e}"));
        }

        info!(%server_id, "server settings updated");
        Ok(())
    }

    pub async fn update_server_settings_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        name: Option<&str>,
        icon_url: Option<&str>,
    ) -> Result<(), String> {
        let (new_name, new_icon) = self
            .organization_service()?
            .update_server(actor, &referenced_server_id(server_id)?, name, icon_url)
            .await
            .map_err(String::from)?;
        let mut server = self
            .servers
            .get_mut(server_id)
            .ok_or("FORBIDDEN: resource unavailable")?;
        server.name = new_name;
        server.icon_url = new_icon;
        Ok(())
    }

    pub async fn update_emoji_settings_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        allow_external: bool,
        shareable: bool,
    ) -> Result<(), String> {
        self.organization_service()?
            .update_emoji_settings(
                actor,
                &referenced_server_id(server_id)?,
                allow_external,
                shareable,
            )
            .await
            .map_err(String::from)
    }

    pub async fn update_member_role_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        target_user_id: &str,
        role: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .update_member_role(
                actor,
                &referenced_server_id(server_id)?,
                target_user_id,
                role,
            )
            .await
            .map_err(String::from)
    }

    pub async fn set_member_avatar_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), String> {
        self.organization_service()?
            .set_member_avatar(actor, &referenced_server_id(server_id)?, avatar_url)
            .await
            .map_err(String::from)?;
        self.broadcast_to_server(
            server_id,
            &ChatEvent::ServerAvatarUpdate {
                server_id: server_id.to_owned(),
                user_id: actor.user_id().as_str().to_owned(),
                avatar_url: avatar_url.map(str::to_owned),
            },
        );
        Ok(())
    }

    /// List servers for a user (by their DB user_id).
    pub async fn list_servers_for_user(&self, user_id: &str) -> Vec<ServerInfo> {
        let mut servers = Vec::new();
        for entry in self.servers.iter() {
            let s = entry.value();
            if !s.member_user_ids.contains(user_id) {
                continue;
            }
            let role = if s.owner_id == user_id {
                Some("owner".to_string())
            } else {
                Some("member".to_string())
            };
            let perms = self.get_effective_permissions(&s.id, None, user_id).await;
            servers.push(ServerInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                icon_url: s.icon_url.clone(),
                member_count: s.member_user_ids.len(),
                role,
                my_permissions: perms.bits() as i64,
            });
        }
        servers
    }

    pub async fn list_servers_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<ServerInfo>, String> {
        self.organization_service()?
            .list_servers_for_actor(actor)
            .await
            .map_err(String::from)
    }

    pub async fn server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(ServerInfo, super::authorization::AuthorizationStamp), String> {
        self.organization_service()?
            .server_for_actor(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)
    }

    pub async fn server_members_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<super::organization::ServerMemberSummary>,
            super::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.organization_service()?
            .server_members_for_actor(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)
    }

    pub async fn list_all_servers_for_admin(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<ServerInfo>, String> {
        self.organization_service()?
            .list_servers_as_admin(actor)
            .await
            .map_err(String::from)
    }

    pub async fn set_system_admin_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        target_user_id: &str,
        is_admin: bool,
    ) -> Result<(), String> {
        self.organization_service()?
            .set_system_admin(actor, target_user_id, is_admin)
            .await
            .map_err(String::from)
    }

    /// List all servers (for system admin).
    pub fn list_all_servers(&self) -> Vec<ServerInfo> {
        self.servers
            .iter()
            .map(|s| ServerInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                icon_url: s.icon_url.clone(),
                member_count: s.member_user_ids.len(),
                role: None,
                my_permissions: 0,
            })
            .collect()
    }

    /// Check if a user is the owner of a server.
    pub fn is_server_owner(&self, server_id: &str, user_id: &str) -> bool {
        self.servers
            .get(server_id)
            .map(|s| s.owner_id == user_id)
            .unwrap_or(false)
    }

    /// Check if a user is a member of a server (in-memory check).
    pub fn user_is_server_member(&self, server_id: &str, user_id: &str) -> bool {
        self.servers
            .get(server_id)
            .map(|s| s.member_user_ids.contains(user_id))
            .unwrap_or(false)
    }

    /// Join a server (persistent membership).
    pub async fn join_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if !self.servers.contains_key(server_id) {
            return Err(format!("No such server: {server_id}"));
        }

        // Check if the user is banned from this server
        if let Some(pool) = &self.db {
            if crate::db::queries::bans::is_banned(pool, server_id, user_id)
                .await
                .unwrap_or(false)
            {
                return Err("You are banned from this server".into());
            }

            crate::db::queries::servers::add_server_member(pool, server_id, user_id, "member")
                .await
                .map_err(|e| format!("Failed to join server: {e}"))?;
        }

        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.insert(user_id.to_string());
        }

        Ok(())
    }

    pub async fn join_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .join_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server
                .member_user_ids
                .insert(actor.user_id().as_str().to_owned());
        }
        Ok(())
    }

    /// Leave a server (remove persistent membership).
    pub async fn leave_server(&self, user_id: &str, server_id: &str) -> Result<(), String> {
        if let Some(pool) = &self.db {
            crate::db::queries::servers::remove_server_member(pool, server_id, user_id)
                .await
                .map_err(|e| format!("Failed to leave server: {e}"))?;
        }

        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(user_id);
        }

        Ok(())
    }

    pub async fn leave_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<(), String> {
        self.organization_service()?
            .leave_server(actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(actor.user_id().as_str());
        }
        Ok(())
    }

    /// Get the role of a user in a server.
    pub async fn get_server_role(&self, server_id: &str, user_id: &str) -> Option<ServerRole> {
        let Some(pool) = &self.db else {
            return None;
        };
        let member = crate::db::queries::servers::get_server_member(pool, server_id, user_id)
            .await
            .ok()
            .flatten()?;
        Some(ServerRole::parse(&member.role))
    }

    /// Look up server_id by server name (for IRC).
    pub fn find_server_by_name(&self, name: &str) -> Option<String> {
        let name_lower = name.to_lowercase();
        if let Some(server_id) = self.server_alias_index.get(&name_lower) {
            return Some(server_id.clone());
        }
        self.servers
            .iter()
            .find(|s| s.name.to_lowercase() == name_lower)
            .map(|s| s.id.clone())
    }

    /// Get a server's name by ID.
    pub fn get_server_name(&self, server_id: &str) -> Option<String> {
        self.servers.get(server_id).map(|s| s.name.clone())
    }

    pub fn get_server_alias(&self, server_id: &str) -> Option<String> {
        self.server_aliases
            .get(server_id)
            .map(|alias| alias.clone())
    }

    /// Resolve an IRC channel through the actor's durable default server and
    /// server/channel aliases, then authorize the resolved stable channel ID.
    pub async fn resolve_irc_server_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_alias: Option<&str>,
    ) -> Result<String, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        auth.validate_actor(actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let server_id = match server_alias {
            Some(alias) => {
                crate::db::queries::aliases::resolve_server_alias(
                    pool,
                    alias.trim_start_matches('#'),
                    actor.user_id().as_str(),
                )
                .await
            }
            None => {
                crate::db::queries::aliases::get_default_server(pool, actor.user_id().as_str())
                    .await
            }
        }
        .map_err(|_| "resource unavailable".to_string())?
        .ok_or_else(|| "resource unavailable".to_string())?;
        super::authorization::AuthorizationService::new(pool.clone())
            .server_members_for_actor(auth, actor, &server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        Ok(server_id)
    }

    pub async fn resolve_irc_channel_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        irc_name: &str,
    ) -> Result<(String, String), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let bare = irc_name.strip_prefix('#').unwrap_or(irc_name);
        let (server_id, channel_alias) =
            if let Some((server_alias, channel_alias)) = bare.split_once('/') {
                let server_id = self
                    .resolve_irc_server_for_actor(actor, Some(server_alias))
                    .await?;
                (server_id, channel_alias)
            } else {
                let server_id = self.resolve_irc_server_for_actor(actor, None).await?;
                (server_id, bare)
            };
        let channel_id =
            crate::db::queries::aliases::resolve_channel_alias(pool, &server_id, channel_alias)
                .await
                .map_err(|_| "resource unavailable".to_string())?
                .ok_or_else(|| "resource unavailable".to_string())?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor(
                auth,
                actor,
                &channel_id,
                super::authorization::ChannelAction::View,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let channel_name: String =
            sqlx::query_scalar("SELECT name FROM channels WHERE id=? AND server_id=?")
                .bind(channel_id)
                .bind(&server_id)
                .fetch_one(pool)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        Ok((server_id, channel_name))
    }

    /// Get the owner user ID for a server.
    pub fn get_server_owner_id(&self, server_id: &str) -> Option<String> {
        self.servers.get(server_id).map(|s| s.owner_id.clone())
    }

    /// Get IRC-style mode string for a channel (e.g., "+ins").
    pub fn get_channel_modes(&self, server_id: &str, channel_name: &str) -> String {
        let key = (server_id.to_string(), channel_name.to_string());
        let Some(channel_id) = self.channel_name_index.get(&key).map(|v| v.clone()) else {
            return "+".to_string();
        };
        let Some(ch) = self.channels.get(&channel_id) else {
            return "+".to_string();
        };
        let mut modes = String::from("+n"); // no external messages (always set)
        if ch.is_private {
            modes.push('i'); // invite-only
        }
        if ch.slowmode_seconds > 0 {
            modes.push('m'); // moderated (closest IRC analog to slowmode)
        }
        modes
    }

    // ── Channel management ──────────────────────────────────────────

    /// Create a channel within a server. Returns the channel ID.
    pub async fn create_channel_in_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        category_id: Option<&str>,
        is_private: bool,
        channel_type: &str,
    ) -> Result<String, String> {
        if !matches!(channel_type, "text" | "forum") {
            return Err("channel type must be text or forum".into());
        }
        let name = normalize_channel_name(name);
        validation::validate_channel_name(&name)?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let channel_id = Uuid::new_v4().to_string();
        self.organization_service()?
            .create_channel(
                &actor,
                super::organization::CreateChannel {
                    server_id: &referenced_server_id(server_id)?,
                    channel_id: &referenced_channel_id(&channel_id)?,
                    name: &name,
                    category_id,
                    is_private,
                    channel_type,
                },
            )
            .await
            .map_err(String::from)?;

        let mut ch = ChannelState::new(channel_id.clone(), server_id.to_string(), name.clone());
        ch.category_id = category_id.map(|s| s.to_string());
        ch.is_private = is_private;
        ch.channel_type = channel_type.to_string();
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.insert(channel_id.clone());
        }
        self.channel_name_index
            .insert((server_id.to_string(), name.clone()), channel_id.clone());
        self.channels.insert(channel_id.clone(), ch);

        Ok(channel_id)
    }

    /// Delete a channel from a server.
    pub async fn delete_channel_in_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.organization_service()?
            .delete_channel(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
            )
            .await
            .map_err(String::from)?;

        self.channels.remove(&channel_id);
        self.channel_name_index
            .remove(&(server_id.to_string(), channel_name));
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.remove(&channel_id);
        }

        Ok(())
    }

    /// Join a channel within a server.
    pub async fn join_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        validation::validate_channel_name(&channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.clone()))
            .map(|id| id.clone())
            .ok_or_else(|| format!("No such channel: {channel_name}"))?;
        if let Some(pool) = &self.db {
            let actor = self
                .get_authenticated_actor(session_id)
                .ok_or_else(|| "resource unavailable".to_string())?;
            let auth = self.auth.get().ok_or("Authentication unavailable")?;
            super::authorization::AuthorizationService::new(pool.clone())
                .authorize_actor(
                    auth,
                    &actor,
                    &channel_id,
                    super::authorization::ChannelAction::View,
                )
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        }

        // Add session to channel
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            channel.members.insert(session_id);
        }

        // Copy the in-memory projection before database hydration so no DashMap
        // guard is held across an await.
        if let Some((topic, mut members)) = self.channels.get(&channel_id).map(|channel| {
            let members = channel
                .members
                .iter()
                .filter_map(|sid| {
                    self.sessions.get(sid).map(|s| MemberInfo {
                        nickname: s.nickname.clone(),
                        avatar_url: s.avatar_url.clone(),
                        server_avatar_url: None,
                        status: None,
                        custom_status: None,
                        status_emoji: None,
                        user_id: s.user_id.clone(),
                        role_ids: Vec::new(),
                    })
                })
                .collect::<Vec<_>>();
            (channel.topic.clone(), members)
        }) {
            if !topic.is_empty() {
                let _ = session.send_guarded(
                    ChatEvent::Topic {
                        server_id: server_id.to_string(),
                        channel: channel_name.clone(),
                        topic,
                    },
                    Some(super::user_session::DeliveryGuard::Channels(vec![
                        channel_id.clone(),
                    ])),
                );
            }

            // Send member list to the joiner. Hydrate server presentation and role
            // assignments here as well as in an explicit GetMembers response so a
            // reconnect cannot transiently erase the authoritative projection.
            if self.db.is_some() {
                self.hydrate_server_member_projections(server_id, &mut members)
                    .await?;
            }

            let joining_member = members
                .iter()
                .find(|member| {
                    session
                        .user_id
                        .as_deref()
                        .is_some_and(|user_id| member.user_id.as_deref() == Some(user_id))
                        || (session.user_id.is_none() && member.nickname == session.nickname)
                })
                .cloned()
                .unwrap_or(MemberInfo {
                    nickname: session.nickname.clone(),
                    avatar_url: session.avatar_url.clone(),
                    server_avatar_url: None,
                    status: None,
                    custom_status: None,
                    status_emoji: None,
                    user_id: session.user_id.clone(),
                    role_ids: Vec::new(),
                });
            self.broadcast_to_channel(
                &channel_id,
                &ChatEvent::Join {
                    nickname: joining_member.nickname,
                    server_id: server_id.to_string(),
                    channel: channel_name.clone(),
                    avatar_url: joining_member.avatar_url,
                    user_id: joining_member.user_id,
                    server_avatar_url: joining_member.server_avatar_url,
                    role_ids: joining_member.role_ids,
                },
                None,
            );

            let _ = session.send_guarded(
                ChatEvent::Names {
                    server_id: server_id.to_string(),
                    channel: channel_name.clone(),
                    members,
                },
                Some(super::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }

        info!(nickname = %session.nickname, %server_id, %channel_name, "joined channel");
        Ok(())
    }

    /// Leave a channel.
    pub fn part_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        reason: Option<String>,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let mut found = false;
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            found = channel.members.remove(&session_id);
        }

        if !found {
            return Err(format!("Not in channel {channel_name}"));
        }

        let part_event = ChatEvent::Part {
            nickname: session.nickname.clone(),
            server_id: server_id.to_string(),
            channel: channel_name.clone(),
            reason,
        };
        let _ = session.send(part_event.clone());
        self.broadcast_to_channel(&channel_id, &part_event, Some(session_id));

        // Remove empty channels from memory (but not from DB)
        self.channels
            .remove_if(&channel_id, |_, ch| ch.members.is_empty());

        info!(nickname = %session.nickname, %server_id, %channel_name, "parted channel");
        Ok(())
    }

    /// Send a message to a channel or user (DM), with optional reply and attachments.
    pub async fn submit_channel_message(
        &self,
        session_id: ConnectionId,
        command: super::messaging::SendMessageCommand<'_>,
        legacy_nonce: Option<&str>,
    ) -> Result<super::messaging::CommandReceipt, super::messaging::MessagingError> {
        let actor = self
            .authenticated_actors
            .get(&session_id)
            .map(|entry| entry.clone())
            .ok_or(super::messaging::MessagingError::Unauthenticated)?;
        let messaging = self
            .messaging
            .get()
            .ok_or(super::messaging::MessagingError::DependencyUnavailable)?;
        let receipt = messaging
            .send_channel_message(&actor, command.clone())
            .await?;

        if let Some(session) = self.sessions.get(&session_id) {
            let id = super::ids::MessageId::from_stored(receipt.message_id.clone())
                .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
            let _ = session.send(ChatEvent::MessageAck {
                id,
                server_id: command.server_id.to_owned(),
                channel: normalize_channel_name(command.channel),
                conversation_id: None,
                request_id: receipt.request_id.clone(),
                client_message_id: receipt.client_message_id.clone(),
                sequence: receipt.sequence.clone(),
                persisted_at: receipt.persisted_at.clone(),
                replayed: receipt.replayed,
                nonce: legacy_nonce.map(str::to_owned),
            });
        }

        if !receipt.replayed {
            let pool = self
                .db
                .as_ref()
                .ok_or(super::messaging::MessagingError::DependencyUnavailable)?;
            match crate::db::queries::messages::get_message_by_id(pool, &receipt.message_id).await {
                Ok(Some(row)) => {
                    if let (Some(channel_id), Some(timestamp)) = (
                        row.channel_id.as_deref(),
                        parse_persisted_timestamp(&row.created_at),
                    ) {
                        let id = super::ids::MessageId::from_stored(row.id.clone())
                            .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
                        let event = ChatEvent::Message {
                            id,
                            server_id: row.server_id.clone(),
                            conversation_id: None,
                            from: row.sender_nick,
                            target: normalize_channel_name(command.channel),
                            content: validation::sanitize_html(&row.content),
                            timestamp,
                            avatar_url: self
                                .sessions
                                .get(&session_id)
                                .and_then(|session| session.avatar_url.clone()),
                            reply_to: None,
                            attachments: None,
                        };
                        let conversation_id: Option<String> =
                            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                                .bind(&receipt.message_id)
                                .fetch_optional(pool)
                                .await
                                .ok()
                                .flatten();
                        if let Some(conversation_id) = conversation_id {
                            self.broadcast_to_channel_guarded(
                                channel_id,
                                &conversation_id,
                                &event,
                                None,
                            );
                        }
                    } else {
                        warn!(message_id = %receipt.message_id, "committed message projection requires replay");
                    }
                }
                Ok(None) => {
                    warn!(message_id = %receipt.message_id, "committed message missing during live projection")
                }
                Err(error) => {
                    warn!(%error, message_id = %receipt.message_id, "committed message projection failed; durable replay remains pending")
                }
            }
        }
        Ok(receipt)
    }

    pub async fn list_direct_conversations(&self, session_id: ConnectionId) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        self.auth
            .get()
            .ok_or("Authentication unavailable")?
            .validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let rows = sqlx::query(
            "SELECT c.id,u.id,u.username,u.avatar_url,MAX(m.created_at), \
                    COUNT(CASE WHEN m.conversation_sequence>COALESCE(rs.conversation_sequence,0) \
                               AND m.sender_id<>? AND m.deleted_at IS NULL THEN 1 END) \
             FROM conversations c \
             JOIN conversation_participants self_cp ON self_cp.conversation_id=c.id \
                 AND self_cp.user_id=? AND self_cp.left_at IS NULL \
             JOIN conversation_participants peer_cp ON peer_cp.conversation_id=c.id \
                 AND peer_cp.user_id<>? AND peer_cp.left_at IS NULL \
             JOIN users u ON u.id=peer_cp.user_id \
             LEFT JOIN messages m ON m.conversation_id=c.id \
             LEFT JOIN read_states rs ON rs.user_id=? AND rs.channel_id=c.id \
             WHERE c.kind='direct' GROUP BY c.id,u.id,u.username,u.avatar_url \
             ORDER BY MAX(m.created_at) DESC,c.created_at DESC,c.id DESC",
        )
        .bind(actor.user_id().as_str())
        .bind(actor.user_id().as_str())
        .bind(actor.user_id().as_str())
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let conversations = rows
            .into_iter()
            .map(|row| DirectConversationInfo {
                id: row.get(0),
                peer_id: row.get(1),
                peer_username: row.get(2),
                peer_avatar_url: row.get(3),
                last_message_at: row.get(4),
                unread_count: row.get::<i64, _>(5).max(0) as u64,
            })
            .collect();
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let _ = session.send_guarded(
            ChatEvent::DirectConversationList { conversations },
            Some(super::user_session::DeliveryGuard::ActorCurrent),
        );
        Ok(())
    }

    pub async fn submit_direct_message(
        &self,
        session_id: ConnectionId,
        command: super::messaging::SendDirectMessageCommand<'_>,
        legacy_nonce: Option<&str>,
    ) -> Result<super::messaging::CommandReceipt, super::messaging::MessagingError> {
        let actor = self.actor_for_session(session_id)?;
        let receipt = self
            .messaging_service()?
            .send_direct_message(&actor, command.clone())
            .await?;
        let pool = self
            .db
            .as_ref()
            .ok_or(super::messaging::MessagingError::DependencyUnavailable)?;
        let conversation_id: String =
            sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=?")
                .bind(&receipt.message_id)
                .fetch_one(pool)
                .await?;
        if let Some(session) = self.sessions.get(&session_id) {
            let id = super::ids::MessageId::from_stored(receipt.message_id.clone())
                .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
            let _ = session.send(ChatEvent::MessageAck {
                id,
                server_id: String::new(),
                channel: command.recipient.to_owned(),
                conversation_id: Some(conversation_id.clone()),
                request_id: receipt.request_id.clone(),
                client_message_id: receipt.client_message_id.clone(),
                sequence: receipt.sequence.clone(),
                persisted_at: receipt.persisted_at.clone(),
                replayed: receipt.replayed,
                nonce: legacy_nonce.map(str::to_owned),
            });
        }
        if !receipt.replayed {
            let row = sqlx::query(
                "SELECT m.sender_nick,m.target_user_id,m.content,m.created_at,u.username, \
                        m.conversation_id \
                 FROM messages m JOIN users u ON u.id=m.target_user_id WHERE m.id=?",
            )
            .bind(&receipt.message_id)
            .fetch_optional(pool)
            .await?
            .ok_or(super::messaging::MessagingError::DependencyUnavailable)?;
            let timestamp = parse_persisted_timestamp(row.get::<&str, _>(3))
                .ok_or(super::messaging::MessagingError::DependencyUnavailable)?;
            let id = super::ids::MessageId::from_stored(receipt.message_id.clone())
                .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
            let target_user_id: String = row.get(1);
            let event = ChatEvent::Message {
                id,
                server_id: None,
                conversation_id: Some(conversation_id.clone()),
                from: row.get(0),
                target: row.get(4),
                content: validation::sanitize_html(row.get(2)),
                timestamp,
                avatar_url: self
                    .sessions
                    .get(&session_id)
                    .and_then(|session| session.avatar_url.clone()),
                reply_to: None,
                attachments: None,
            };
            for session in self.sessions.iter() {
                if session.id != session_id
                    && (session.user_id.as_deref() == Some(target_user_id.as_str())
                        || session.user_id.as_deref() == Some(actor.user_id().as_str()))
                {
                    let _ = session.send_guarded(
                        event.clone(),
                        Some(super::user_session::DeliveryGuard::Conversations(vec![
                            conversation_id.clone(),
                        ])),
                    );
                }
            }
        }
        Ok(receipt)
    }

    pub async fn submit_edit_message(
        &self,
        session_id: ConnectionId,
        command: super::messaging::EditMessageCommand<'_>,
    ) -> Result<super::messaging::CommandReceipt, super::messaging::MessagingError> {
        let actor = self.actor_for_session(session_id)?;
        let mutation = self
            .messaging_service()?
            .edit_message(&actor, command)
            .await?;
        if !mutation.receipt.replayed {
            let id = super::ids::MessageId::from_stored(mutation.receipt.message_id.clone())
                .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
            let channel = self
                .resolve_channel_name_from_id(&mutation.channel_id)
                .unwrap_or(mutation.channel_id.clone());
            self.broadcast_to_channel_guarded(
                &mutation.channel_id,
                &mutation.conversation_id,
                &ChatEvent::MessageEdit {
                    id,
                    server_id: mutation.server_id,
                    channel,
                    content: validation::sanitize_html(mutation.content.as_deref().unwrap_or("")),
                    edited_at: Utc::now(),
                },
                None,
            );
        }
        self.send_committed_receipt(session_id, &mutation.receipt);
        Ok(mutation.receipt)
    }

    pub async fn submit_delete_message(
        &self,
        session_id: ConnectionId,
        command: super::messaging::EntityCommand<'_>,
    ) -> Result<super::messaging::CommandReceipt, super::messaging::MessagingError> {
        let actor = self.actor_for_session(session_id)?;
        let mutation = self
            .messaging_service()?
            .delete_message(&actor, command)
            .await?;
        if !mutation.receipt.replayed {
            let id = super::ids::MessageId::from_stored(mutation.receipt.message_id.clone())
                .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
            let channel = self
                .resolve_channel_name_from_id(&mutation.channel_id)
                .unwrap_or(mutation.channel_id.clone());
            self.broadcast_to_channel_guarded(
                &mutation.channel_id,
                &mutation.conversation_id,
                &ChatEvent::MessageDelete {
                    id,
                    server_id: mutation.server_id,
                    channel,
                },
                None,
            );
        }
        self.send_committed_receipt(session_id, &mutation.receipt);
        Ok(mutation.receipt)
    }

    pub async fn submit_reaction(
        &self,
        session_id: ConnectionId,
        command: super::messaging::ReactionCommand<'_>,
        add: bool,
    ) -> Result<super::messaging::CommandReceipt, super::messaging::MessagingError> {
        let actor = self.actor_for_session(session_id)?;
        let mutation = self
            .messaging_service()?
            .change_reaction(&actor, command, add)
            .await?;
        if !mutation.receipt.replayed {
            let message_id =
                super::ids::MessageId::from_stored(mutation.receipt.message_id.clone())
                    .map_err(|_| super::messaging::MessagingError::DependencyUnavailable)?;
            let channel = self
                .resolve_channel_name_from_id(&mutation.channel_id)
                .unwrap_or(mutation.channel_id.clone());
            let nickname = self
                .sessions
                .get(&session_id)
                .map(|session| session.nickname.clone())
                .unwrap_or_default();
            let event = if add {
                ChatEvent::ReactionAdd {
                    message_id,
                    server_id: mutation.server_id,
                    channel,
                    user_id: mutation.actor_id,
                    nickname,
                    emoji: mutation.emoji.unwrap_or_default(),
                }
            } else {
                ChatEvent::ReactionRemove {
                    message_id,
                    server_id: mutation.server_id,
                    channel,
                    user_id: mutation.actor_id,
                    nickname,
                    emoji: mutation.emoji.unwrap_or_default(),
                }
            };
            self.broadcast_to_channel_guarded(
                &mutation.channel_id,
                &mutation.conversation_id,
                &event,
                None,
            );
        }
        self.send_committed_receipt(session_id, &mutation.receipt);
        Ok(mutation.receipt)
    }

    pub async fn submit_mark_read(
        &self,
        session_id: ConnectionId,
        command: super::messaging::ReadCommand<'_>,
    ) -> Result<super::messaging::CommandReceipt, super::messaging::MessagingError> {
        let actor = self.actor_for_session(session_id)?;
        let receipt = self.messaging_service()?.mark_read(&actor, command).await?;
        self.send_committed_receipt(session_id, &receipt);
        Ok(receipt)
    }

    fn actor_for_session(
        &self,
        session_id: ConnectionId,
    ) -> Result<crate::auth::authority::Actor, super::messaging::MessagingError> {
        self.authenticated_actors
            .get(&session_id)
            .map(|actor| actor.clone())
            .ok_or(super::messaging::MessagingError::Unauthenticated)
    }

    fn messaging_service(
        &self,
    ) -> Result<&super::messaging::MessagingService, super::messaging::MessagingError> {
        self.messaging
            .get()
            .ok_or(super::messaging::MessagingError::DependencyUnavailable)
    }

    fn integration_service(&self) -> Result<super::integrations::IntegrationService, String> {
        Ok(super::integrations::IntegrationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
            self.integration_vault
                .get()
                .ok_or("Integration credential vault unavailable")?
                .clone(),
        ))
    }

    pub fn configure_integration_vault(
        &self,
        vault: Arc<crate::secrets::SecretVault>,
    ) -> Result<(), String> {
        self.integration_vault
            .set(vault)
            .map_err(|_| "Integration credential vault already configured".into())
    }

    fn send_committed_receipt(
        &self,
        session_id: ConnectionId,
        receipt: &super::messaging::CommandReceipt,
    ) {
        if let Some(session) = self.sessions.get(&session_id) {
            let _ = session.send(ChatEvent::CommandCommitted {
                receipt: receipt.clone(),
            });
        }
    }

    /// Legacy in-memory compatibility path retained only while non-message callers migrate.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn send_message(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target: &str,
        content: &str,
        reply_to_id: Option<&str>,
        attachment_ids: Option<&[String]>,
        nonce: Option<&str>,
    ) -> Result<(), String> {
        validation::validate_message_with_limit(content, self.max_message_length)?;
        let content = &validation::sanitize_html(content);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        if !self.message_limiter.check(&session.nickname) {
            return Err("Rate limit exceeded. Please slow down.".into());
        }

        // Enforce timeout: timed-out users cannot send messages
        if let Some(pool) = &self.db
            && let Some(ref uid) = session.user_id
        {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let uid = uid.clone();
            let timed_out = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Ok(Some(until)) =
                        crate::db::queries::moderation::get_member_timeout(&pool, &srv, &uid).await
                        && let Ok(timeout_dt) =
                            chrono::NaiveDateTime::parse_from_str(&until, "%Y-%m-%d %H:%M:%S")
                    {
                        let timeout_utc = timeout_dt.and_utc();
                        return timeout_utc > chrono::Utc::now();
                    }
                    false
                })
            });
            if timed_out {
                return Err("You are timed out and cannot send messages".into());
            }
        }

        // Enforce slow mode: check per-channel cooldown.
        // Uses both a DB query and an in-memory DashMap cache to prevent
        // concurrent requests (e.g. two browser tabs) from bypassing the check.
        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let tgt = target.to_string();
            let sender_uid = session
                .user_id
                .clone()
                .unwrap_or_else(|| session.nickname.clone());
            let slowmode_map = &self.slowmode_last_sent;
            let slow_err = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Ok(Some(ch)) =
                        crate::db::queries::channels::get_channel_by_name(&pool, &srv, &tgt).await
                        && ch.slowmode_seconds > 0
                    {
                        let cooldown_dur =
                            std::time::Duration::from_secs(ch.slowmode_seconds as u64);
                        let cache_key = (sender_uid.clone(), ch.id.clone());

                        // Check the in-memory cache first (catches concurrent sends)
                        if let Some(last_instant) = slowmode_map.get(&cache_key)
                            && last_instant.elapsed() < cooldown_dur
                        {
                            return Some(format!(
                                "Slow mode: wait {} seconds between messages",
                                ch.slowmode_seconds
                            ));
                        }

                        // Also check DB (catches sends from before this process started)
                        if let Ok(Some(last)) =
                            crate::db::queries::messages::get_last_user_message_time(
                                &pool,
                                &ch.id,
                                &sender_uid,
                            )
                            .await
                            && let Ok(last_dt) =
                                chrono::NaiveDateTime::parse_from_str(&last, "%Y-%m-%d %H:%M:%S")
                        {
                            let last_utc = last_dt.and_utc();
                            let cooldown = chrono::Duration::seconds(ch.slowmode_seconds as i64);
                            if chrono::Utc::now() - last_utc < cooldown {
                                return Some(format!(
                                    "Slow mode: wait {} seconds between messages",
                                    ch.slowmode_seconds
                                ));
                            }
                        }

                        // Both checks passed — record this send in the in-memory cache
                        slowmode_map.insert(cache_key, Instant::now());
                    }
                    None
                })
            });
            if let Some(err) = slow_err {
                return Err(err);
            }
        }

        // Evaluate automod rules (keyword, mention_spam, link_filter)
        if let Some(pool) = &self.db {
            let pool = pool.clone();
            let srv = server_id.to_string();
            let content_clone = content.to_string();
            let automod_err = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let rules = crate::db::queries::automod::get_enabled_rules(&pool, &srv)
                        .await
                        .unwrap_or_default();
                    for rule in rules {
                        let triggered = match rule.rule_type.as_str() {
                            "keyword" => {
                                // Config: {"words":["bad","spam"]}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    if let Some(words) =
                                        config.get("words").and_then(|w| w.as_array())
                                    {
                                        let lower = content_clone.to_lowercase();
                                        let msg_words: Vec<&str> =
                                            lower.split(|c: char| !c.is_alphanumeric()).collect();
                                        words.iter().any(|w| {
                                            w.as_str().is_some_and(|kw| {
                                                let kw_lower = kw.to_lowercase();
                                                msg_words.iter().any(|mw| *mw == kw_lower)
                                            })
                                        })
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            "mention_spam" => {
                                // Config: {"max_mentions":5}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    let max = config
                                        .get("max_mentions")
                                        .and_then(|m| m.as_i64())
                                        .unwrap_or(5)
                                        as usize;
                                    let mention_count = content_clone.matches('@').count();
                                    mention_count > max
                                } else {
                                    false
                                }
                            }
                            "link_filter" => {
                                // Config: {"block_all":true}
                                if let Ok(config) =
                                    serde_json::from_str::<serde_json::Value>(&rule.config)
                                {
                                    let block_all = config
                                        .get("block_all")
                                        .and_then(|b| b.as_bool())
                                        .unwrap_or(false);
                                    if block_all {
                                        content_clone.contains("http://")
                                            || content_clone.contains("https://")
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };
                        if triggered {
                            return Some(format!("Message blocked by automod rule: {}", rule.name));
                        }
                    }
                    None
                })
            });
            if let Some(err) = automod_err {
                return Err(err);
            }
        }

        // Build reply info if replying to a message
        let reply_to: Option<ReplyInfo> = if let Some(ref_id) = reply_to_id {
            if let Some(pool) = &self.db {
                // Synchronous lookup via block_in_place — reply info is needed before broadcast
                let pool = pool.clone();
                let ref_id = ref_id.to_string();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        match crate::db::queries::messages::get_message_by_id(&pool, &ref_id).await
                        {
                            Ok(Some(row)) => Some(ReplyInfo {
                                id: row.id,
                                from: row.sender_nick,
                                content_preview: row.content.chars().take(100).collect::<String>(),
                            }),
                            _ => None,
                        }
                    })
                })
            } else {
                None
            }
        } else {
            None
        };

        // Look up attachment metadata if attachment_ids provided
        let attachments: Option<Vec<super::events::AttachmentInfo>> = if let Some(ids) =
            attachment_ids
            && !ids.is_empty()
        {
            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let ids = ids.to_vec();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let infos =
                            crate::db::queries::attachments::get_attachments_by_ids(&pool, &ids)
                                .await
                                .unwrap_or_default();
                        if infos.is_empty() {
                            None
                        } else {
                            Some(
                                infos
                                    .into_iter()
                                    .map(|a| super::events::AttachmentInfo {
                                        id: a.id.clone(),
                                        filename: a.original_filename,
                                        content_type: a.content_type,
                                        file_size: a.file_size,
                                        url: format!("/api/uploads/{}", a.id),
                                    })
                                    .collect(),
                            )
                        }
                    })
                })
            } else {
                None
            }
        } else {
            None
        };

        let msg_id = super::ids::MessageId::from(Uuid::new_v4());
        let event = ChatEvent::Message {
            id: msg_id.clone(),
            server_id: Some(server_id.to_string()),
            conversation_id: None,
            from: session.nickname.clone(),
            target: target.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            avatar_url: session.avatar_url.clone(),
            reply_to: reply_to.clone(),
            attachments: attachments.clone(),
        };

        if target.starts_with('#') {
            let channel_name = normalize_channel_name(target);
            let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

            let channel = self
                .channels
                .get(&channel_id)
                .ok_or(format!("No such channel: {channel_name}"))?;

            // Check if thread is archived
            if channel.archived {
                return Err("This thread is archived and no longer accepts messages".to_string());
            }

            if !channel.members.contains(&session_id) {
                return Err(format!("You are not in channel {channel_name}"));
            }

            // Private channel access control: require VIEW_CHANNELS even if user is in-memory member
            let ch_is_private = channel.is_private;
            drop(channel);

            if ch_is_private {
                if let Some(ref uid) = session.user_id
                    && self.db.is_some()
                {
                    let has_view = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let perms = self
                                .get_effective_permissions(server_id, Some(&channel_id), uid)
                                .await;
                            perms.contains(crate::engine::permissions::Permissions::VIEW_CHANNELS)
                        })
                    });
                    if !has_view {
                        return Err(
                            "You do not have permission to access this private channel".to_string()
                        );
                    }
                } else if session.user_id.is_none() {
                    return Err("Authentication required to access private channels".to_string());
                }
            }

            // Check SEND_MESSAGES permission (only when DB is available for role/override lookups)
            if self.db.is_some() {
                let sender_user_id = session
                    .user_id
                    .clone()
                    .unwrap_or_else(|| session_id.to_string());
                let perms = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(self.get_effective_permissions(
                        server_id,
                        Some(&channel_id),
                        &sender_user_id,
                    ))
                });
                if !perms.contains(crate::engine::permissions::Permissions::SEND_MESSAGES) {
                    return Err(
                        "You do not have permission to send messages in this channel".to_string(),
                    );
                }
            }

            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let id = msg_id.to_string();
                let srv = server_id.to_string();
                let ch = channel_id.clone();
                let sid = session_id.to_string();
                let nick = session.nickname.clone();
                let uid = session.user_id.clone().unwrap_or_else(|| sid.clone());
                let msg = content.to_string();
                let reply_id = reply_to_id.map(|s| s.to_string());
                let att_ids = attachment_ids.map(|ids| ids.to_vec());
                tokio::spawn(async move {
                    let params = crate::db::queries::messages::InsertMessageParams {
                        id: &id,
                        server_id: &srv,
                        channel_id: &ch,
                        sender_id: &uid,
                        sender_nick: &nick,
                        content: &msg,
                        reply_to_id: reply_id.as_deref(),
                    };
                    if let Err(e) =
                        crate::db::queries::messages::insert_message(&pool, &params).await
                    {
                        error!(error = %e, "failed to persist message");
                    }
                    // Link attachments to the message (use user_id, not session_id)
                    if let Some(att_ids) = att_ids
                        && let Err(e) =
                            crate::db::queries::attachments::link_attachments_to_message(
                                &pool, &id, &att_ids, &uid,
                            )
                            .await
                    {
                        error!(error = %e, "failed to link attachments");
                    }
                });
            }

            self.broadcast_to_channel(&channel_id, &event, Some(session_id));

            // Send MessageAck back to the sender with the server-generated message ID
            if let Some(sender_session) = self.sessions.get(&session_id) {
                let _ = sender_session.send(ChatEvent::MessageAck {
                    id: msg_id.clone(),
                    server_id: server_id.to_string(),
                    channel: target.to_string(),
                    conversation_id: None,
                    request_id: nonce.unwrap_or_default().to_owned(),
                    client_message_id: nonce.unwrap_or_default().to_owned(),
                    sequence: String::new(),
                    persisted_at: Utc::now().to_rfc3339(),
                    replayed: false,
                    nonce: nonce.map(|s| s.to_string()),
                });
            }

            // Async link embed unfurling — extract URLs and resolve OG metadata
            let urls = super::embeds::extract_urls(content);
            if !urls.is_empty()
                && let Some(pool) = &self.db
            {
                let pool = pool.clone();
                let client = crate::egress::ControlledHttpClient::internet()
                    .expect("static controlled HTTP client limits are valid");
                let server_id_owned = server_id.to_string();
                let channel_name_owned = channel_name.clone();
                let channel_id_owned = channel_id.clone();
                // Collect senders for channel members before spawning
                let member_sessions: Vec<Arc<UserSession>> =
                    if let Some(channel) = self.channels.get(&channel_id) {
                        channel
                            .members
                            .iter()
                            .filter_map(|sid| self.sessions.get(sid).map(|s| s.clone()))
                            .collect()
                    } else {
                        vec![]
                    };
                tokio::spawn(async move {
                    let mut embeds = Vec::new();
                    for url in urls {
                        // Check cache first
                        if let Ok(Some(cached)) =
                            crate::db::queries::embeds::get_cached_embed(&pool, &url).await
                        {
                            embeds.push(super::events::EmbedInfo {
                                url: cached.url,
                                title: cached.title,
                                description: cached.description,
                                image_url: cached.image_url,
                                site_name: cached.site_name,
                            });
                            continue;
                        }
                        // Unfurl
                        if let Some(info) = super::embeds::unfurl_url(&client, &url).await {
                            let _ = crate::db::queries::embeds::upsert_embed(
                                &pool,
                                &info.url,
                                info.title.as_deref(),
                                info.description.as_deref(),
                                info.image_url.as_deref(),
                                info.site_name.as_deref(),
                            )
                            .await;
                            embeds.push(info);
                        }
                    }
                    if !embeds.is_empty() {
                        let embed_event = ChatEvent::MessageEmbed {
                            message_id: msg_id.clone(),
                            server_id: server_id_owned,
                            channel: channel_name_owned,
                            embeds,
                        };
                        for session in &member_sessions {
                            let _ = session.send_guarded(
                                embed_event.clone(),
                                Some(super::user_session::DeliveryGuard::Channels(vec![
                                    channel_id_owned.clone(),
                                ])),
                            );
                        }
                    }
                });
            }
        } else {
            // DM
            let target_session_id = self
                .nick_to_session
                .get(&crate::auth::authority::rfc1459_casefold(target))
                .ok_or(format!("No such user: {target}"))?;

            if let Some(pool) = &self.db {
                let pool = pool.clone();
                let id = msg_id.to_string();
                let sender_uid = session
                    .user_id
                    .clone()
                    .unwrap_or_else(|| session_id.to_string());
                let nick = session.nickname.clone();
                let target_uid = self
                    .sessions
                    .get(target_session_id.value())
                    .and_then(|s| s.user_id.clone())
                    .unwrap_or_else(|| target_session_id.value().to_string());
                let msg = content.to_string();
                tokio::spawn(async move {
                    if let Err(e) = crate::db::queries::messages::insert_dm(
                        &pool,
                        &id,
                        &sender_uid,
                        &nick,
                        &target_uid,
                        &msg,
                    )
                    .await
                    {
                        error!(error = %e, "failed to persist DM");
                    }
                });
            }

            let target_user_id = self
                .sessions
                .get(target_session_id.value())
                .and_then(|target_session| target_session.user_id.clone());
            if let Some(target_user_id) = target_user_id {
                if let Some(connections) = self.user_connections.get(&target_user_id) {
                    for connection_id in connections.iter() {
                        if let Some(target_session) = self.sessions.get(connection_id) {
                            let _ = target_session.send_guarded(
                                event.clone(),
                                Some(super::user_session::DeliveryGuard::ActorCurrent),
                            );
                        }
                    }
                }
            } else if let Some(target_session) = self.sessions.get(target_session_id.value()) {
                let _ = target_session.send_guarded(
                    event,
                    Some(super::user_session::DeliveryGuard::ActorCurrent),
                );
            }

            // Send MessageAck back to the DM sender
            if let Some(sender_session) = self.sessions.get(&session_id) {
                let _ = sender_session.send(ChatEvent::MessageAck {
                    id: msg_id.clone(),
                    server_id: String::new(),
                    channel: target.to_string(),
                    conversation_id: None,
                    request_id: nonce.unwrap_or_default().to_owned(),
                    client_message_id: nonce.unwrap_or_default().to_owned(),
                    sequence: String::new(),
                    persisted_at: Utc::now().to_rfc3339(),
                    replayed: false,
                    nonce: nonce.map(|s| s.to_string()),
                });
            }
        }

        Ok(())
    }

    /// Set the topic for a channel.
    pub async fn set_topic(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        topic: String,
    ) -> Result<(), String> {
        validation::validate_topic(&topic)?;
        let topic = validation::sanitize_html(&topic);
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel = self
            .channels
            .get_mut(&channel_id)
            .ok_or(format!("No such channel: {channel_name}"))?;

        if !channel.members.contains(&session_id) {
            return Err(format!("You are not in channel {channel_name}"));
        }

        drop(channel);

        if let Some(pool) = &self.db {
            let actor = self
                .get_authenticated_actor(session_id)
                .ok_or_else(|| "resource unavailable".to_string())?;
            let auth = self.auth.get().ok_or("Authentication unavailable")?;
            super::organization::OrganizationService::new(
                pool.clone(),
                auth.clone(),
                self.write_admission
                    .as_ref()
                    .ok_or("Write admission unavailable")?
                    .clone(),
            )
            .set_topic(
                &actor,
                &referenced_channel_id(&channel_id)?,
                &topic,
                &session.nickname,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        }
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            channel.topic.clone_from(&topic);
            channel.topic_set_by = Some(session.nickname.clone());
            channel.topic_set_at = Some(Utc::now());
        }

        let event = ChatEvent::TopicChange {
            server_id: server_id.to_string(),
            channel: channel_name,
            set_by: session.nickname.clone(),
            topic,
        };
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Fetch message history for a channel, including edits, replies, and reactions.
    pub async fn fetch_history(
        &self,
        server_id: &str,
        channel_name: &str,
        before: Option<&str>,
        limit: i64,
        actor: &crate::auth::authority::Actor,
    ) -> Result<
        (
            Vec<HistoryMessage>,
            bool,
            super::authorization::AuthorizationStamp,
        ),
        String,
    > {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let stamp = super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_stamped(
                auth,
                actor,
                &channel_id,
                super::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        let rows = crate::db::queries::messages::fetch_channel_history(
            pool,
            &channel_id,
            before,
            limit + 1,
        )
        .await
        .map_err(|e| format!("Failed to fetch history: {e}"))?;

        let has_more = rows.len() as i64 > limit;
        let rows: Vec<_> = rows.into_iter().take(limit as usize).collect();

        // Collect message IDs for batch reaction lookup
        let msg_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();

        // Fetch reactions for all messages in batch
        let reaction_rows =
            crate::db::queries::messages::get_reactions_for_messages(pool, &msg_ids)
                .await
                .unwrap_or_default();

        // Group reactions by message_id -> emoji -> user_ids
        let mut reaction_map: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Vec<String>>,
        > = std::collections::HashMap::new();
        for r in &reaction_rows {
            reaction_map
                .entry(r.message_id.clone())
                .or_default()
                .entry(r.emoji.clone())
                .or_default()
                .push(r.user_id.clone());
        }

        // Collect reply_to_ids for batch lookup
        let reply_ids: Vec<String> = rows.iter().filter_map(|r| r.reply_to_id.clone()).collect();
        let mut reply_map: std::collections::HashMap<String, ReplyInfo> =
            std::collections::HashMap::new();
        if !reply_ids.is_empty() {
            for rid in &reply_ids {
                if let Ok(Some((id, from, content))) = sqlx::query_as::<_, (String, String, String)>(
                    "SELECT id,sender_nick,CASE WHEN deleted_at IS NULL THEN content ELSE '' END FROM messages \
                     WHERE id=? AND conversation_id=(SELECT id FROM conversations WHERE channel_id=?)",
                )
                .bind(rid)
                .bind(&channel_id)
                .fetch_optional(pool)
                .await
                {
                    reply_map.insert(
                        id.clone(),
                        ReplyInfo {
                            id,
                            from,
                            content_preview: content.chars().take(100).collect(),
                        },
                    );
                }
            }
        }

        // Fetch attachments for all messages in batch
        let attachment_rows =
            crate::db::queries::attachments::get_attachments_for_messages(pool, &msg_ids)
                .await
                .unwrap_or_default();

        // Group attachments by message_id
        let mut attachment_map: std::collections::HashMap<
            String,
            Vec<super::events::AttachmentInfo>,
        > = std::collections::HashMap::new();
        for a in &attachment_rows {
            if let Some(ref mid) = a.message_id {
                attachment_map.entry(mid.clone()).or_default().push(
                    super::events::AttachmentInfo {
                        id: a.id.clone(),
                        filename: a.original_filename.clone(),
                        content_type: a.content_type.clone(),
                        file_size: a.file_size,
                        url: format!("/api/uploads/{}", a.id),
                    },
                );
            }
        }

        let mut rich_embed_map = std::collections::HashMap::new();
        let mut component_map = std::collections::HashMap::new();
        if !msg_ids.is_empty() {
            let mut builder = sqlx::QueryBuilder::new(
                "SELECT id,rich_embeds_json,components_json FROM messages WHERE id IN (",
            );
            let mut separated = builder.separated(",");
            for id in &msg_ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            if let Ok(stored) = builder.build().fetch_all(pool).await {
                use sqlx::Row;
                for item in stored {
                    let id: String = item.get(0);
                    if let Some(value) = item.get::<Option<&str>, _>(1)
                        && let Ok(parsed) = serde_json::from_str(value)
                    {
                        rich_embed_map.insert(id.clone(), parsed);
                    }
                    if let Some(value) = item.get::<Option<&str>, _>(2)
                        && let Ok(parsed) = serde_json::from_str(value)
                    {
                        component_map.insert(id, parsed);
                    }
                }
            }
        }

        let messages: Vec<HistoryMessage> = rows
            .into_iter()
            .map(|row| -> Result<HistoryMessage, String> {
                let reactions = reaction_map.get(&row.id).map(|emoji_map| {
                    emoji_map
                        .iter()
                        .map(|(emoji, user_ids)| ReactionGroup {
                            emoji: emoji.clone(),
                            count: user_ids.len(),
                            user_ids: user_ids.clone(),
                        })
                        .collect()
                });
                let reply_to = row
                    .reply_to_id
                    .as_ref()
                    .and_then(|rid| reply_map.get(rid).cloned());
                let edited_at = row
                    .edited_at
                    .as_deref()
                    .map(|value| {
                        parse_persisted_timestamp(value).ok_or_else(|| {
                            "Stored message has an invalid edited timestamp".to_string()
                        })
                    })
                    .transpose()?;
                let timestamp = parse_persisted_timestamp(&row.created_at).ok_or_else(|| {
                    "Stored message has an invalid creation timestamp".to_string()
                })?;
                let attachments = attachment_map.remove(&row.id);
                let rich_embeds = rich_embed_map.remove(&row.id);
                let components = component_map.remove(&row.id);

                Ok(HistoryMessage {
                    id: super::ids::MessageId::from_stored(row.id)
                        .map_err(|_| "Stored message has an invalid identifier".to_string())?,
                    from: row.sender_nick,
                    content: row.content,
                    timestamp,
                    edited_at,
                    reply_to,
                    reactions,
                    attachments,
                    embeds: None,
                    rich_embeds,
                    components,
                })
            })
            .collect::<Result<_, _>>()?;

        Ok((messages, has_more, stamp))
    }

    /// List all channels in a server.
    pub fn list_channels(&self, server_id: &str) -> Vec<ChannelInfo> {
        self.channels
            .iter()
            .filter(|ch| ch.server_id == server_id)
            .map(|entry| ChannelInfo {
                id: entry.id.clone(),
                conversation_id: channel_conversation_id(&entry.id),
                server_id: entry.server_id.clone(),
                name: entry.name.clone(),
                topic: entry.topic.clone(),
                member_count: entry.member_count(),
                category_id: entry.category_id.clone(),
                position: entry.position,
                is_private: entry.is_private,
                channel_type: entry.channel_type.clone(),
                thread_parent_message_id: entry.thread_parent_message_id.clone(),
                archived: entry.archived,
                slowmode_seconds: entry.slowmode_seconds,
                is_nsfw: entry.is_nsfw,
            })
            .collect()
    }

    /// List only channels visible to the current database-backed member snapshot.
    pub async fn list_visible_channels(
        &self,
        server_id: &str,
        user_id: &str,
    ) -> Result<Vec<ChannelInfo>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = super::authorization::AuthorizationService::new(pool.clone())
            .visible_channels(user_id, server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        Ok(rows
            .into_iter()
            .map(|row| ChannelInfo {
                member_count: self
                    .channels
                    .get(&row.id)
                    .map_or(0, |state| state.member_count()),
                conversation_id: channel_conversation_id(&row.id),
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                topic: row.topic,
                category_id: row.category_id,
                position: row.position,
                is_private: row.is_private != 0,
                channel_type: row.channel_type,
                thread_parent_message_id: row.thread_parent_message_id,
                archived: row.archived != 0,
                slowmode_seconds: row.slowmode_seconds,
                is_nsfw: row.is_nsfw != 0,
            })
            .collect())
    }

    pub async fn list_visible_channels_for_actor(
        &self,
        server_id: &str,
        actor: &crate::auth::authority::Actor,
    ) -> Result<(Vec<ChannelInfo>, super::authorization::AuthorizationStamp), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let (rows, stamp) = super::authorization::AuthorizationService::new(pool.clone())
            .visible_channels_for_actor(auth, actor, server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let channels = rows
            .into_iter()
            .map(|row| ChannelInfo {
                member_count: self
                    .channels
                    .get(&row.id)
                    .map_or(0, |state| state.member_count()),
                conversation_id: channel_conversation_id(&row.id),
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                topic: row.topic,
                category_id: row.category_id,
                position: row.position,
                is_private: row.is_private != 0,
                channel_type: row.channel_type,
                thread_parent_message_id: row.thread_parent_message_id,
                archived: row.archived != 0,
                slowmode_seconds: row.slowmode_seconds,
                is_nsfw: row.is_nsfw != 0,
            })
            .collect();
        Ok((channels, stamp))
    }

    pub async fn send_visible_channel_list(
        &self,
        session_id: ConnectionId,
        server_id: String,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (channels, stamp) = self
            .list_visible_channels_for_actor(&server_id, &actor)
            .await?;
        let session = self
            .get_session(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        if !session.send_guarded(
            ChatEvent::ChannelList {
                server_id,
                channels,
            },
            Some(super::user_session::DeliveryGuard::Stamps(vec![stamp])),
        ) {
            return Err("delivery unavailable".into());
        }
        Ok(())
    }

    /// Get members of a channel.
    pub fn get_members(
        &self,
        server_id: &str,
        channel_name: &str,
    ) -> Result<Vec<MemberInfo>, String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let channel = self
            .channels
            .get(&channel_id)
            .ok_or(format!("No such channel: {channel_name}"))?;

        Ok(channel
            .members
            .iter()
            .filter_map(|sid| {
                self.sessions.get(sid).map(|s| MemberInfo {
                    nickname: s.nickname.clone(),
                    avatar_url: s.avatar_url.clone(),
                    server_avatar_url: None,
                    status: None,
                    custom_status: None,
                    status_emoji: None,
                    user_id: s.user_id.clone(),
                    role_ids: Vec::new(),
                })
            })
            .collect())
    }

    pub async fn get_visible_members(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(Vec<MemberInfo>, super::authorization::AuthorizationStamp), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let normalized = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &normalized)?;
        let stamp = super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_stamped(
                auth,
                actor,
                &channel_id,
                super::authorization::ChannelAction::View,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let mut members = self.get_members(server_id, &normalized)?;
        self.hydrate_server_member_projections(server_id, &mut members)
            .await?;
        Ok((members, stamp))
    }

    async fn hydrate_server_member_projections(
        &self,
        server_id: &str,
        members: &mut [MemberInfo],
    ) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let identities: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT user_id,nickname,avatar_url FROM server_members WHERE server_id=?",
        )
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let identities: std::collections::HashMap<_, _> = identities
            .into_iter()
            .map(|(user_id, nickname, avatar)| (user_id, (nickname, avatar)))
            .collect();
        let role_rows: Vec<(String, String)> =
            sqlx::query_as("SELECT user_id,role_id FROM user_roles WHERE server_id=?")
                .bind(server_id)
                .fetch_all(pool)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        let mut role_ids: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (user_id, role_id) in role_rows {
            role_ids.entry(user_id).or_default().push(role_id);
        }
        for member in members {
            if let Some(user_id) = member.user_id.as_deref()
                && let Some((nickname, avatar)) = identities.get(user_id)
            {
                if let Some(nickname) = nickname {
                    member.nickname.clone_from(nickname);
                }
                member.server_avatar_url.clone_from(avatar);
            }
            if let Some(user_id) = member.user_id.as_deref() {
                member.role_ids = role_ids.remove(user_id).unwrap_or_default();
            }
        }
        Ok(())
    }

    // ── Message editing & deletion ─────────────────────────────────

    /// Edit a message's content. Only the sender or a moderator+ can edit.
    #[cfg(test)]
    pub async fn edit_message(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        new_content: &str,
    ) -> Result<(), String> {
        validation::validate_message_with_limit(new_content, self.max_message_length)?;
        let new_content = &validation::sanitize_html(new_content);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;
        // Only the sender can edit their own messages, unless user has MANAGE_MESSAGES
        let sender_id = session
            .user_id
            .as_deref()
            .ok_or("Authentication required to edit messages")?;
        if msg.sender_id != sender_id {
            let server_id = msg.server_id.as_deref().ok_or("Message has no server")?;
            let channel_id = msg.channel_id.as_deref().ok_or("Message has no channel")?;
            let perms = self
                .get_effective_permissions(server_id, Some(channel_id), sender_id)
                .await;
            if !perms.contains(Permissions::MANAGE_MESSAGES) {
                return Err("You can only edit your own messages".into());
            }
        }

        crate::db::queries::messages::update_message_content(pool, message_id, new_content)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        // Find the channel name for the event
        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::MessageEdit {
            id: super::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id: server_id.clone(),
            channel: channel_name,
            content: new_content.to_string(),
            edited_at: Utc::now(),
        };

        // Broadcast to the channel (including sender)
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Delete a message (soft delete). Sender can delete own, moderator+ can delete any.
    #[cfg(test)]
    pub async fn delete_message(
        &self,
        session_id: ConnectionId,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let sender_id = session
            .user_id
            .as_deref()
            .ok_or("Authentication required to delete messages")?;
        let is_sender = msg.sender_id == sender_id;

        if !is_sender {
            // Check if user has MANAGE_MESSAGES permission
            let server_id = msg.server_id.as_deref().ok_or("Message has no server")?;
            let channel_id_ref = msg.channel_id.as_deref().ok_or("Message has no channel")?;
            let perms = self
                .get_effective_permissions(server_id, Some(channel_id_ref), sender_id)
                .await;
            if !perms.contains(Permissions::MANAGE_MESSAGES) {
                return Err("You can only delete your own messages".into());
            }
        }

        crate::db::queries::messages::soft_delete_message(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::MessageDelete {
            id: super::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id,
            channel: channel_name,
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    // ── Reactions ────────────────────────────────────────────────────

    /// Add a reaction to a message.
    #[cfg(test)]
    pub async fn add_reaction(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;
        let user_id = session.user_id.as_deref().unwrap_or(&session.nickname);

        crate::db::queries::messages::add_reaction(pool, message_id, user_id, emoji)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::ReactionAdd {
            message_id: super::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id,
            channel: channel_name,
            user_id: user_id.to_string(),
            nickname: session.nickname.clone(),
            emoji: emoji.to_string(),
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Remove a reaction from a message.
    #[cfg(test)]
    pub async fn remove_reaction(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let user_id = session.user_id.as_deref().unwrap_or(&session.nickname);

        crate::db::queries::messages::remove_reaction(pool, message_id, user_id, emoji)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::ReactionRemove {
            message_id: super::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id,
            channel: channel_name,
            user_id: user_id.to_string(),
            nickname: session.nickname.clone(),
            emoji: emoji.to_string(),
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    // ── Typing indicators ────────────────────────────────────────────

    /// Broadcast a typing indicator to a channel.
    pub fn send_typing(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let event = ChatEvent::TypingStart {
            server_id: server_id.to_string(),
            channel: channel_name,
            nickname: session.nickname.clone(),
        };

        self.broadcast_to_channel(&channel_id, &event, Some(session_id));

        Ok(())
    }

    // ── Read state ────────────────────────────────────────────────────

    /// Mark a channel as read for a user, up to a specific message ID.
    #[cfg(test)]
    pub async fn mark_read(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let stamp = super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_stamped(
                auth,
                &actor,
                &channel_id,
                super::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        crate::db::queries::messages::mark_channel_read(pool, user_id, &channel_id, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        if !self.authorization_stamp_is_current(&actor, &stamp).await {
            return Err("resource unavailable".into());
        }

        Ok(())
    }

    /// Get unread counts for all channels in a server for a user.
    pub async fn get_unread_counts(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<
        (
            Vec<super::events::UnreadCount>,
            Vec<super::authorization::AuthorizationStamp>,
        ),
        String,
    > {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows = crate::db::queries::messages::get_unread_counts(pool, user_id, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        // Map channel_id -> channel_name
        let authorization = super::authorization::AuthorizationService::new(pool.clone());
        let mut counts = Vec::new();
        let mut stamps = Vec::new();
        for r in rows {
            let Ok(stamp) = authorization
                .authorize_actor_stamped(
                    auth,
                    &actor,
                    &r.channel_id,
                    super::authorization::ChannelAction::ReadHistory,
                )
                .await
            else {
                continue;
            };
            stamps.push(stamp);
            if let Some(name) = self.channels.get(&r.channel_id).map(|ch| ch.name.clone()) {
                counts.push(super::events::UnreadCount {
                    channel_name: name,
                    count: r.unread_count,
                });
            }
        }
        for stamp in &stamps {
            if !self.authorization_stamp_is_current(&actor, stamp).await {
                return Err("resource unavailable".into());
            }
        }
        Ok((counts, stamps))
    }

    // ── Roles ────────────────────────────────────────────────────────

    /// Get effective permissions for a user in a channel.
    pub async fn get_effective_permissions(
        &self,
        server_id: &str,
        channel_id: Option<&str>,
        user_id: &str,
    ) -> Permissions {
        let Some(pool) = &self.db else {
            return Permissions::empty();
        };
        super::authorization::AuthorizationService::new(pool.clone())
            .effective_permissions(user_id, server_id, channel_id)
            .await
            .unwrap_or_else(|error| {
                error!(%error, "authorization failed closed");
                Permissions::empty()
            })
    }

    /// Check that a user has a required permission. Returns Ok(user_id) or Err(message).
    pub async fn require_permission(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: Option<&str>,
        required: Permissions,
    ) -> Result<String, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();

        let perms = self
            .get_effective_permissions(server_id, channel_id, &user_id)
            .await;

        if perms.contains(required) {
            Ok(user_id)
        } else {
            Err("FORBIDDEN: insufficient permissions".into())
        }
    }

    /// List roles for a server.
    pub async fn list_roles(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(i64, Vec<RoleInfo>, Vec<MemberRoleInfo>), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .list_roles(&actor, &referenced_server_id(server_id)?)
        .await?;
        let mut connection = pool.acquire().await.map_err(|_| "resource unavailable")?;
        let version: i64 =
            sqlx::query_scalar("SELECT role_projection_version FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *connection)
                .await
                .map_err(|_| "resource unavailable")?;
        let roles: Vec<crate::db::models::RoleRow> =
            sqlx::query_as("SELECT * FROM roles WHERE server_id=? ORDER BY position DESC")
                .bind(server_id)
                .fetch_all(&mut *connection)
                .await
                .map_err(|_| "resource unavailable")?;
        let assignments: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT sm.user_id,ur.role_id FROM server_members sm LEFT JOIN user_roles ur ON ur.server_id=sm.server_id AND ur.user_id=sm.user_id WHERE sm.server_id=? ORDER BY sm.user_id,ur.role_id",
        )
        .bind(server_id)
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| "resource unavailable")?;
        Ok((
            version,
            roles.into_iter().map(role_row_to_info).collect(),
            group_member_roles(assignments),
        ))
    }

    /// Create a custom role in a server.
    pub async fn create_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: i64,
    ) -> Result<RoleInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let role_id = Uuid::new_v4().to_string();
        let role = super::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .create_role(
            &actor,
            &referenced_server_id(server_id)?,
            &role_id,
            name,
            color,
            permissions,
        )
        .await?;

        Ok(role_row_to_info(role))
    }

    /// Update a custom role.
    pub async fn update_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: i64,
    ) -> Result<RoleInfo, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let role = super::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .update_role(
            &actor,
            &referenced_server_id(server_id)?,
            role_id,
            name,
            color,
            permissions,
        )
        .await?;
        Ok(role_row_to_info(role))
    }

    /// Delete a custom role.
    pub async fn delete_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        role_id: &str,
    ) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .delete_role(&actor, &referenced_server_id(server_id)?, role_id)
        .await
        .map_err(Into::into)
    }

    /// Get the highest role position for a user in a server.
    /// Server owner gets i32::MAX. Returns 0 if no roles found (base @everyone level).
    pub async fn get_user_highest_role_position(&self, server_id: &str, user_id: &str) -> i32 {
        if self.is_server_owner(server_id, user_id) {
            return i32::MAX;
        }
        let Some(pool) = &self.db else { return 0 };
        crate::db::queries::roles::get_user_roles(pool, server_id, user_id)
            .await
            .unwrap_or_default()
            .iter()
            .map(|r| r.position)
            .max()
            .unwrap_or(0)
    }

    /// Validate role hierarchy: actor must have a higher position than target role.
    pub async fn check_role_hierarchy(
        &self,
        server_id: &str,
        actor_user_id: &str,
        target_role_id: &str,
    ) -> Result<(), String> {
        // Server owner bypasses hierarchy checks
        if self.is_server_owner(server_id, actor_user_id) {
            return Ok(());
        }
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let target_role = crate::db::queries::roles::get_role(pool, target_role_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Role not found")?;
        let actor_highest = self
            .get_user_highest_role_position(server_id, actor_user_id)
            .await;
        if actor_highest <= target_role.position {
            return Err("You cannot manage a role at or above your own highest role".to_string());
        }
        Ok(())
    }

    fn evict_user_from_server_subscriptions(&self, server_id: &str, user_id: &str) {
        let sessions: std::collections::HashSet<_> = self
            .sessions
            .iter()
            .filter(|session| session.user_id.as_deref() == Some(user_id))
            .map(|session| *session.key())
            .collect();
        for mut channel in self.channels.iter_mut() {
            if channel.server_id == server_id {
                channel
                    .members
                    .retain(|session_id| !sessions.contains(session_id));
            }
        }
    }

    /// Assign a role to a user. Enforces role hierarchy.
    pub async fn assign_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .set_member_role(
            &actor,
            &referenced_server_id(server_id)?,
            target_user_id,
            role_id,
            true,
        )
        .await
        .map_err(Into::into)
    }

    /// Remove a role from a user. Enforces role hierarchy.
    pub async fn remove_role(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .set_member_role(
            &actor,
            &referenced_server_id(server_id)?,
            target_user_id,
            role_id,
            false,
        )
        .await
        .map_err(Into::into)
    }

    /// Publish one authoritative role projection while holding write admission,
    /// preventing an older post-commit notification from overtaking a newer edit.
    pub async fn broadcast_role_snapshot(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        changed_user_id: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let (_permit, mut tx) = writes.begin().await.map_err(|error| error.to_string())?;
        super::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        )
        .server_actor_permissions_in(&mut tx, auth, &actor, server_id)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let version: i64 =
            sqlx::query_scalar("SELECT role_projection_version FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| "resource unavailable")?;
        let roles: Vec<crate::db::models::RoleRow> =
            sqlx::query_as("SELECT * FROM roles WHERE server_id=? ORDER BY position DESC")
                .bind(server_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        let role_ids = if let Some(user_id) = changed_user_id {
            Some(sqlx::query_scalar(
                "SELECT role_id FROM user_roles WHERE server_id=? AND user_id=? ORDER BY role_id",
            ).bind(server_id).bind(user_id).fetch_all(&mut *tx).await
                .map_err(|_| "resource unavailable".to_string())?)
        } else {
            None
        };
        tx.commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        self.broadcast_to_server(
            server_id,
            &ChatEvent::RoleList {
                server_id: server_id.to_owned(),
                version,
                roles: roles.into_iter().map(role_row_to_info).collect(),
                member_roles: None,
            },
        );
        if let (Some(user_id), Some(role_ids)) = (changed_user_id, role_ids) {
            self.broadcast_to_server(
                server_id,
                &ChatEvent::MemberRoleUpdate {
                    server_id: server_id.to_owned(),
                    version,
                    user_id: user_id.to_owned(),
                    role_ids,
                },
            );
        }
        Ok(())
    }

    pub async fn list_channel_permission_overrides(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
    ) -> Result<Vec<super::events::ChannelPermissionOverrideInfo>, String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        self.organization_service()?
            .list_channel_overrides(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(channel_id)?,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn set_channel_permission_override(
        &self,
        session_id: ConnectionId,
        update: super::organization::ChannelOverrideUpdate<'_>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        self.organization_service()?
            .set_channel_override(&actor, update)
            .await
            .map_err(Into::into)
    }

    pub async fn delete_channel_permission_override(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
        target_type: &str,
        target_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        self.organization_service()?
            .delete_channel_override(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(channel_id)?,
                target_type,
                target_id,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn broadcast_channel_permission_overrides(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("resource unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut tx) = writes.begin().await.map_err(|error| error.to_string())?;
        super::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        )
        .require_server_actor_in(
            &mut tx,
            self.auth.get().ok_or("Authentication unavailable")?,
            &actor,
            server_id,
            Permissions::MANAGE_CHANNELS,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let rows: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT id,channel_id,target_type,target_id,allow_bits,deny_bits \
             FROM channel_permission_overrides WHERE channel_id=? \
             ORDER BY target_type,target_id",
        )
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        tx.commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let overrides = rows
            .into_iter()
            .map(
                |(id, channel_id, target_type, target_id, allow_bits, deny_bits)| {
                    super::events::ChannelPermissionOverrideInfo {
                        id,
                        channel_id,
                        target_type,
                        target_id,
                        allow_bits,
                        deny_bits,
                    }
                },
            )
            .collect();
        let event = ChatEvent::ChannelPermissionOverrideList {
            server_id: server_id.to_owned(),
            channel_id: channel_id.to_owned(),
            overrides,
        };
        let Some(server) = self.servers.get(server_id) else {
            return Ok(());
        };
        let member_ids: Vec<String> = server.member_user_ids.iter().cloned().collect();
        drop(server);
        for session in self.sessions.iter() {
            if session
                .user_id
                .as_ref()
                .is_some_and(|user_id| member_ids.contains(user_id))
            {
                let _ = session.send_guarded(
                    event.clone(),
                    Some(super::user_session::DeliveryGuard::ServerPermissions(vec![
                        (server_id.to_owned(), Permissions::MANAGE_CHANNELS),
                    ])),
                );
            }
        }
        Ok(())
    }

    // ── Categories ──────────────────────────────────────────────────

    /// List categories for a server.
    pub async fn list_categories(&self, server_id: &str) -> Result<Vec<CategoryInfo>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = crate::db::queries::categories::list_categories(pool, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        Ok(rows.into_iter().map(category_row_to_info).collect())
    }

    /// Create a channel category.
    pub async fn create_category(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let cat_id = Uuid::new_v4().to_string();
        super::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .create_category(&actor, &referenced_server_id(server_id)?, &cat_id, name)
        .await
        .map_err(Into::into)
    }

    /// Update a channel category name.
    pub async fn update_category(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        category_id: &str,
        name: &str,
    ) -> Result<CategoryInfo, String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .update_category(&actor, &referenced_server_id(server_id)?, category_id, name)
        .await
        .map_err(Into::into)
    }

    /// Delete a channel category.
    pub async fn delete_category(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        category_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .delete_category(&actor, &referenced_server_id(server_id)?, category_id)
        .await?;
        // Channels referencing this category get NULL (ON DELETE SET NULL)
        // Update in-memory state
        for mut ch in self.channels.iter_mut() {
            if ch.category_id.as_deref() == Some(category_id) {
                ch.category_id = None;
            }
        }
        Ok(())
    }

    // ── Channel organization ────────────────────────────────────────

    /// Reorder channels: update position and category for a batch of channels.
    pub async fn reorder_channels(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        updates: &[ChannelPositionInfo],
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        super::organization::OrganizationService::new(
            pool.clone(),
            auth.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        )
        .reorder_channels(&actor, &referenced_server_id(server_id)?, updates)
        .await?;
        for update in updates {
            // Update in-memory state
            if let Some(mut ch) = self.channels.get_mut(&update.id) {
                ch.position = update.position;
                ch.category_id.clone_from(&update.category_id);
            }
        }
        Ok(())
    }

    // ── Profiles ───────────────────────────────────────────────────

    /// Get a user's full profile.
    pub async fn get_user_profile(
        &self,
        actor: &crate::auth::authority::Actor,
        user_id: &str,
    ) -> Result<
        (
            super::events::UserProfileInfo,
            Option<super::authorization::AuthorizationStamp>,
        ),
        String,
    > {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let mut transaction = pool.begin().await.map_err(|_| "resource unavailable")?;
        auth.validate_actor_in(&mut transaction, actor)
            .await
            .map_err(|_| "resource unavailable")?;
        let shared_server: Option<(String, i64)> = if actor.user_id().as_str() == user_id {
            None
        } else {
            sqlx::query_as(
                "SELECT s.id,s.authorization_version FROM servers s \
                 JOIN server_members requester ON requester.server_id=s.id AND requester.user_id=? \
                 JOIN server_members target ON target.server_id=s.id AND target.user_id=? \
                 WHERE NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id IN (?,?)) \
                 ORDER BY s.id LIMIT 1",
            )
            .bind(actor.user_id().as_str())
            .bind(user_id)
            .bind(actor.user_id().as_str())
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| "resource unavailable")?
        };
        if actor.user_id().as_str() != user_id && shared_server.is_none() {
            return Err("resource unavailable".into());
        }
        let row = sqlx::query(
            "SELECT u.id,u.username,u.avatar_url,p.bio,p.pronouns,p.banner_url,u.created_at \
             FROM users u LEFT JOIN user_profiles p ON p.user_id=u.id \
             WHERE u.id=? AND u.disabled_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?
        .ok_or_else(|| "resource unavailable".to_string())?;
        let profile = super::events::UserProfileInfo {
            user_id: row.get(0),
            username: row.get(1),
            avatar_url: row.get(2),
            bio: row.get(3),
            pronouns: row.get(4),
            banner_url: row.get(5),
            created_at: row.get(6),
        };
        let stamp = shared_server.map(|(server_id, server_version)| {
            super::authorization::AuthorizationStamp {
                server_id,
                server_version,
                channel_versions: Vec::new(),
            }
        });
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable")?;
        Ok((profile, stamp))
    }

    pub fn broadcast_profile_update(&self, profile: super::events::UserProfileInfo) {
        for server in self.servers.iter() {
            if !server.member_user_ids.contains(&profile.user_id) {
                continue;
            }
            let mut notified = std::collections::HashSet::new();
            for channel_id in &server.channel_ids {
                if let Some(channel) = self.channels.get(channel_id) {
                    for session_id in &channel.members {
                        if notified.insert(*session_id)
                            && let Some(session) = self.sessions.get(session_id)
                        {
                            let _ = session.send_guarded(
                                ChatEvent::UserProfile {
                                    profile: profile.clone(),
                                },
                                Some(super::user_session::DeliveryGuard::ServerMembership(vec![
                                    server.id.clone(),
                                ])),
                            );
                        }
                    }
                }
            }
        }
    }

    fn profile_sync_service(
        &self,
    ) -> Result<super::profile_sync::ProfileSyncService, super::profile_sync::ProfileSyncError>
    {
        Ok(super::profile_sync::ProfileSyncService::new(
            self.db
                .as_ref()
                .ok_or(super::profile_sync::ProfileSyncError::DependencyUnavailable)?
                .clone(),
            self.auth
                .get()
                .ok_or(super::profile_sync::ProfileSyncError::DependencyUnavailable)?
                .clone(),
            self.write_admission
                .as_ref()
                .ok_or(super::profile_sync::ProfileSyncError::DependencyUnavailable)?
                .clone(),
        ))
    }

    pub async fn verified_atproto_profile_did(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<String, super::profile_sync::ProfileSyncError> {
        self.profile_sync_service()?.verified_did(actor).await
    }

    pub async fn apply_atproto_profile_sync(
        &self,
        actor: &crate::auth::authority::Actor,
        expected_did: &str,
        profile: &super::profile_sync::BlueskyProfileSyncInput<'_>,
    ) -> Result<super::events::UserProfileInfo, super::profile_sync::ProfileSyncError> {
        let updated = self
            .profile_sync_service()?
            .apply(actor, expected_did, profile)
            .await?;
        self.broadcast_profile_update(updated.clone());
        Ok(updated)
    }

    pub async fn atproto_identity_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        user_id: &str,
    ) -> Result<
        (
            Option<super::profile_sync::AtprotoIdentity>,
            Option<super::authorization::AuthorizationStamp>,
        ),
        super::profile_sync::ProfileSyncError,
    > {
        self.profile_sync_service()?
            .identity_for_actor(actor, user_id)
            .await
    }

    pub async fn atproto_sync_enabled_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<bool, super::profile_sync::ProfileSyncError> {
        self.profile_sync_service()?.sync_enabled(actor).await
    }

    pub async fn set_atproto_sync_enabled_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        enabled: bool,
    ) -> Result<(), super::profile_sync::ProfileSyncError> {
        self.profile_sync_service()?
            .set_sync_enabled(actor, enabled)
            .await
    }

    pub async fn request_atproto_publication(
        &self,
        actor: &crate::auth::authority::Actor,
        message_id: &str,
    ) -> Result<crate::db::queries::atproto::AtprotoPublication, AtprotoPublicationError> {
        use crate::db::queries::atproto::PublicationRequestError;
        crate::db::queries::atproto::request_publication(
            self.write_admission
                .as_ref()
                .ok_or(PublicationRequestError::Unavailable)?,
            &crate::engine::authorization::AuthorizationService::new(
                self.db
                    .as_ref()
                    .ok_or(PublicationRequestError::Unavailable)?
                    .clone(),
            ),
            self.auth
                .get()
                .ok_or(PublicationRequestError::Unavailable)?,
            actor,
            message_id,
        )
        .await
        .map_err(AtprotoPublicationError::from)
    }

    pub async fn list_atproto_publications(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<crate::db::queries::atproto::AtprotoPublicationStatus>, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        self.auth
            .get()
            .ok_or("Authentication unavailable")?
            .validate_actor(actor)
            .await
            .map_err(|error| error.to_string())?;
        crate::db::queries::atproto::list_publications(pool, actor.user_id().as_str())
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn retry_atproto_publication(
        &self,
        actor: &crate::auth::authority::Actor,
        publication_id: &str,
    ) -> Result<crate::db::queries::atproto::AtprotoPublication, AtprotoPublicationError> {
        use crate::db::queries::atproto::PublicationRequestError;
        crate::db::queries::atproto::retry_publication(
            self.write_admission
                .as_ref()
                .ok_or(PublicationRequestError::Unavailable)?,
            &crate::engine::authorization::AuthorizationService::new(
                self.db
                    .as_ref()
                    .ok_or(PublicationRequestError::Unavailable)?
                    .clone(),
            ),
            self.auth
                .get()
                .ok_or(PublicationRequestError::Unavailable)?,
            actor,
            publication_id,
        )
        .await
        .map_err(AtprotoPublicationError::from)
    }

    pub async fn atproto_channel_publication_policy(
        &self,
        actor: &crate::auth::authority::Actor,
        channel_id: &str,
    ) -> Result<crate::db::queries::atproto::AtprotoChannelPublicationPolicy, String> {
        let admission = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = admission.begin().await.map_err(|e| e.to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let authorization = crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        );
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                channel_id,
                super::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|e| e.to_string())?;
        let row = sqlx::query(
            "SELECT c.is_private,c.visibility_repair_required,c.parent_channel_id,c.channel_type,
                    c.atproto_publication_enabled,COALESCE(g.enabled,0)
             FROM channels c LEFT JOIN atproto_publication_grants g
               ON g.channel_id=c.id AND g.user_id=? WHERE c.id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(channel_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Channel unavailable")?;
        let eligible = row.get::<i64, _>(0) == 0
            && row.get::<i64, _>(1) == 0
            && row.get::<Option<String>, _>(2).is_none()
            && !matches!(
                row.get::<String, _>(3).as_str(),
                "public_thread" | "private_thread"
            );
        let policy = crate::db::queries::atproto::AtprotoChannelPublicationPolicy {
            channel_id: channel_id.to_owned(),
            eligible,
            channel_enabled: row.get::<i64, _>(4) == 1,
            user_granted: row.get::<i64, _>(5) == 1,
        };
        transaction.commit().await.map_err(|e| e.to_string())?;
        Ok(policy)
    }

    pub async fn configure_atproto_channel(
        &self,
        actor: &crate::auth::authority::Actor,
        channel_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let admission = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = admission.begin().await.map_err(|e| e.to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let authorization = crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        );
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                channel_id,
                super::authorization::ChannelAction::Manage,
            )
            .await
            .map_err(|e| e.to_string())?;
        let changed = sqlx::query(
            "UPDATE channels SET atproto_publication_enabled=?,authorization_version=authorization_version+1
             WHERE id=? AND is_private=0 AND visibility_repair_required=0
               AND parent_channel_id IS NULL
               AND channel_type NOT IN ('public_thread','private_thread')",
        )
        .bind(i64::from(enabled))
        .bind(channel_id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
        if changed.rows_affected() != 1 {
            return Err("Channel is not eligible for publication".into());
        }
        if !enabled {
            sqlx::query("UPDATE atproto_publications SET status='cancelled',safe_error_code='channel_disabled',updated_at=datetime('now') WHERE source_message_id IN (SELECT id FROM messages WHERE channel_id=?) AND status IN ('pending','update_pending')")
                .bind(channel_id).execute(&mut *transaction).await.map_err(|e| e.to_string())?;
        }
        transaction.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn set_atproto_publication_grant(
        &self,
        actor: &crate::auth::authority::Actor,
        channel_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let admission = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = admission.begin().await.map_err(|e| e.to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let authorization = crate::engine::authorization::AuthorizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
        );
        for action in [
            super::authorization::ChannelAction::View,
            super::authorization::ChannelAction::ReadHistory,
        ] {
            authorization
                .authorize_actor_in(&mut transaction, auth, actor, channel_id, action)
                .await
                .map_err(|e| e.to_string())?;
        }
        let eligible: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND is_private=0
              AND atproto_publication_enabled=1 AND visibility_repair_required=0
              AND parent_channel_id IS NULL
              AND channel_type NOT IN ('public_thread','private_thread'))",
        )
        .bind(channel_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
        if !eligible {
            return Err("Channel is not eligible for publication".into());
        }
        sqlx::query(
            "INSERT INTO atproto_publication_grants(user_id,channel_id,enabled)
             VALUES(?,?,?)
             ON CONFLICT(user_id,channel_id) DO UPDATE SET
               enabled=excluded.enabled,grant_version=atproto_publication_grants.grant_version+1,
               updated_at=datetime('now')",
        )
        .bind(actor.user_id().as_str())
        .bind(channel_id)
        .bind(i64::from(enabled))
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
        if !enabled {
            sqlx::query("UPDATE atproto_publications SET status='cancelled',safe_error_code='grant_revoked',updated_at=datetime('now') WHERE user_id=? AND source_message_id IN (SELECT id FROM messages WHERE channel_id=?) AND status IN ('pending','update_pending')")
                .bind(actor.user_id().as_str()).bind(channel_id).execute(&mut *transaction).await.map_err(|e| e.to_string())?;
        }
        transaction.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Utility ─────────────────────────────────────────────────────

    /// Get a reference to the database pool (if configured).
    pub fn db(&self) -> Option<&SqlitePool> {
        self.db.as_ref()
    }

    /// Check if a nickname is available.
    pub fn is_nick_available(&self, nickname: &str) -> bool {
        !self
            .nick_to_session
            .contains_key(&crate::auth::authority::rfc1459_casefold(nickname))
    }

    /// Look up a session ID by nickname. Returns None if no session with that nick exists.
    pub fn get_session_id_by_nick(&self, nickname: &str) -> Option<ConnectionId> {
        self.nick_to_session
            .get(&crate::auth::authority::rfc1459_casefold(nickname))
            .map(|r| *r)
    }

    /// Get the user_id for a session. Returns None if session not found or has no user.
    pub fn get_session_user_id(&self, session_id: ConnectionId) -> Option<String> {
        self.sessions
            .get(&session_id)
            .and_then(|s| s.user_id.clone())
    }

    /// Get (server_id, channel_name) pairs for all channels a session is in.
    pub fn get_session_channels(&self, session_id: ConnectionId) -> Vec<(String, String)> {
        if !self.sessions.contains_key(&session_id) {
            return vec![];
        }
        self.channels
            .iter()
            .filter(|ch| ch.members.contains(&session_id))
            .map(|ch| (ch.server_id.clone(), ch.name.clone()))
            .collect()
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: ConnectionId) -> Option<Arc<UserSession>> {
        self.sessions.get(&session_id).map(|s| s.clone())
    }

    /// Get the database pool (if configured).
    pub fn get_db(&self) -> Option<SqlitePool> {
        self.db.clone()
    }

    fn community_service(&self) -> Result<super::community_service::CommunityService, String> {
        Ok(super::community_service::CommunityService::new(
            self.db.clone().ok_or("No database configured")?,
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        ))
    }

    fn moderation_service(&self) -> Result<super::moderation::ModerationService, String> {
        Ok(super::moderation::ModerationService::new(
            self.db.clone().ok_or_else(moderation_dependency)?,
            self.auth.get().ok_or_else(moderation_dependency)?.clone(),
            self.write_admission
                .as_ref()
                .ok_or_else(moderation_dependency)?
                .clone(),
        ))
    }

    fn organization_service(&self) -> Result<super::organization::OrganizationService, String> {
        Ok(super::organization::OrganizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        ))
    }

    fn media_service(&self) -> Result<super::media_service::MediaService, String> {
        Ok(super::media_service::MediaService::new(
            self.db
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: media dependency unavailable")?
                .clone(),
            self.auth
                .get()
                .ok_or("DEPENDENCY_UNAVAILABLE: media dependency unavailable")?
                .clone(),
            self.write_admission
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: media dependency unavailable")?
                .clone(),
        ))
    }

    fn account_service(&self) -> Result<super::account::AccountService, String> {
        Ok(super::account::AccountService::new(
            self.db
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: account dependency unavailable")?
                .clone(),
            self.auth
                .get()
                .ok_or("DEPENDENCY_UNAVAILABLE: account dependency unavailable")?
                .clone(),
            self.write_admission
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: account dependency unavailable")?
                .clone(),
        ))
    }

    pub async fn authorize_media_upload(
        &self,
        actor: &crate::auth::authority::Actor,
        target: super::media_service::UploadTarget<'_>,
        instance_max_bytes: u64,
    ) -> Result<super::media_service::AuthorizedUpload, String> {
        self.media_service()?
            .authorize_upload(actor, target, instance_max_bytes)
            .await
    }

    pub async fn reserve_media_upload(
        &self,
        actor: &crate::auth::authority::Actor,
        plan: super::media_service::AuthorizedUpload,
        request: super::media_service::UploadReservation<'_>,
    ) -> Result<crate::media::MediaUpload, crate::media::MediaError> {
        let service = self
            .media_service()
            .map_err(|_| crate::media::MediaError::Invalid)?;
        service.reserve_upload(actor, plan, request).await
    }

    pub async fn update_server_icon_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        icon_url: &str,
    ) -> Result<(), String> {
        self.media_service()?
            .update_server_icon(actor, server_id, icon_url)
            .await?;
        self.load_servers_from_db()
            .await
            .map_err(|_| "DEPENDENCY_UNAVAILABLE: server state refresh failed".to_string())
    }

    pub async fn update_member_avatar_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        avatar_url: &str,
    ) -> Result<(), String> {
        let update = self
            .media_service()?
            .update_member_avatar(actor, server_id, avatar_url)
            .await?;
        let display_name = update.nickname.clone().unwrap_or(update.username);
        self.broadcast_server_member_identity(
            server_id,
            actor.user_id().as_str(),
            update.nickname,
            display_name,
            Some(update.avatar_url),
        );
        Ok(())
    }

    pub async fn create_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        name: &str,
        image_url: &str,
    ) -> Result<super::media_service::CreatedEmoji, String> {
        self.media_service()?
            .create_emoji(actor, server_id, name, image_url)
            .await
    }

    pub async fn list_server_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<super::media_service::EmojiAsset>,
            super::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.media_service()?
            .list_server_emoji(actor, server_id)
            .await
    }

    pub async fn list_user_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        target_server_id: &str,
    ) -> Result<
        (
            Vec<super::media_service::EmojiAsset>,
            super::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.media_service()?
            .list_user_emoji(actor, target_server_id)
            .await
    }

    pub async fn delete_emoji_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        emoji_id: &str,
    ) -> Result<bool, String> {
        self.media_service()?
            .delete_emoji(actor, server_id, emoji_id)
            .await
    }

    pub async fn create_sticker_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        name: &str,
        image_url: &str,
        description: Option<&str>,
    ) -> Result<super::media_service::CreatedSticker, String> {
        self.media_service()?
            .create_sticker(actor, server_id, name, image_url, description)
            .await
    }

    pub async fn list_server_stickers_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
    ) -> Result<
        (
            Vec<super::media_service::StickerAsset>,
            super::authorization::AuthorizationStamp,
        ),
        String,
    > {
        self.media_service()?
            .list_server_stickers(actor, server_id)
            .await
    }

    pub async fn delete_sticker_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        server_id: &str,
        sticker_id: &str,
    ) -> Result<bool, String> {
        self.media_service()?
            .delete_sticker(actor, server_id, sticker_id)
            .await
    }

    pub async fn update_profile_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        update: super::media_service::ProfileUpdate<'_>,
    ) -> Result<(), String> {
        self.media_service()?.update_profile(actor, update).await?;
        let (profile, _) = self
            .get_user_profile(actor, actor.user_id().as_str())
            .await
            .map_err(|_| "DEPENDENCY_UNAVAILABLE: profile refresh failed".to_string())?;
        self.broadcast_profile_update(profile);
        Ok(())
    }

    pub async fn authorized_media_download(
        &self,
        actor: &crate::auth::authority::Actor,
        attachment_id: &str,
    ) -> Result<super::media_service::AuthorizedDownload, String> {
        self.media_service()?
            .authorized_download(actor, attachment_id)
            .await
    }

    pub async fn media_download_is_still_authorized(
        &self,
        actor: &crate::auth::authority::Actor,
        attachment_id: &str,
    ) -> bool {
        let Ok(service) = self.media_service() else {
            return false;
        };
        service
            .download_is_still_authorized(actor, attachment_id)
            .await
    }

    pub async fn delete_unattached_upload_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        attachment_id: &str,
    ) -> Result<bool, String> {
        self.media_service()?
            .delete_unattached_upload(actor, attachment_id)
            .await
    }

    pub async fn list_server_folders_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<super::account::ServerFolder>, String> {
        self.account_service()?.list_server_folders(actor).await
    }

    pub async fn current_account_profile(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Option<super::account::AccountProfile>, String> {
        self.account_service()?.current_profile(actor).await
    }

    pub async fn public_account_profile(
        &self,
        nickname: &str,
    ) -> Result<Option<super::account::PublicAccountProfile>, String> {
        self.account_service()?.public_profile(nickname).await
    }

    pub async fn list_irc_tokens_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<super::account::IrcToken>, String> {
        self.account_service()?.list_irc_tokens(actor).await
    }

    pub async fn public_invite_preview(
        &self,
        code: &str,
    ) -> Result<
        Option<super::community_service::PublicInvitePreview>,
        super::community_service::PublicInvitePreviewError,
    > {
        self.community_service()
            .map_err(|_| super::community_service::PublicInvitePreviewError::DependencyUnavailable)?
            .public_invite_preview(code)
            .await
    }

    pub async fn discover_public_servers(
        &self,
        category: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<crate::db::models::ServerRow>, String> {
        self.community_service()?
            .discover_public(category, limit, offset)
            .await
            .map_err(String::from)
    }

    pub async fn replace_server_folders_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        folders: &[super::account::ServerFolder],
    ) -> Result<(), String> {
        self.account_service()?
            .replace_server_folders(actor, folders)
            .await
    }

    /// Resolve a channel name within a server to its channel ID.
    pub fn resolve_channel_id(
        &self,
        server_id: &str,
        channel_name: &str,
    ) -> Result<String, String> {
        self.channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|r| r.clone())
            .ok_or(format!("No such channel: {channel_name}"))
    }

    /// Broadcast an event to all members of a channel, optionally excluding one session.
    fn broadcast_to_channel(
        &self,
        channel_id: &str,
        event: &ChatEvent,
        exclude: Option<ConnectionId>,
    ) {
        let Some(channel) = self.channels.get(channel_id) else {
            return;
        };

        for member_id in &channel.members {
            if Some(*member_id) == exclude {
                continue;
            }
            let channel_guard_id = match event {
                ChatEvent::ThreadCreate { thread, .. } | ChatEvent::ThreadUpdate { thread, .. } => {
                    thread.id.clone()
                }
                _ => channel_id.to_owned(),
            };
            if let Some(session) = self.sessions.get(member_id)
                && !session.send_guarded(
                    event.clone(),
                    Some(super::user_session::DeliveryGuard::Channels(vec![
                        channel_guard_id,
                    ])),
                )
            {
                warn!(%member_id, "failed to send event to session (channel closed)");
            }
        }
    }

    fn broadcast_to_channel_guarded(
        &self,
        channel_id: &str,
        conversation_id: &str,
        event: &ChatEvent,
        exclude: Option<ConnectionId>,
    ) {
        let Some(channel) = self.channels.get(channel_id) else {
            return;
        };
        for member_id in &channel.members {
            if Some(*member_id) == exclude {
                continue;
            }
            if let Some(session) = self.sessions.get(member_id)
                && !session.send_guarded(
                    event.clone(),
                    Some(super::user_session::DeliveryGuard::Conversations(vec![
                        conversation_id.to_owned(),
                    ])),
                )
            {
                warn!(%member_id, "guarded channel delivery overflowed");
            }
        }
    }

    // ── Presence ─────────────────────────────────────────────

    /// Update a user's presence and broadcast to members of shared servers.
    pub async fn set_presence(
        &self,
        session_id: ConnectionId,
        status: &str,
        custom_status: Option<&str>,
        status_emoji: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "Not authenticated".to_string())?;
        let user_id = actor.user_id().as_str().to_owned();

        // Validate status
        match status {
            "online" | "idle" | "dnd" | "invisible" => {}
            _ => return Err("Invalid status. Must be: online, idle, dnd, invisible".into()),
        }
        if custom_status.is_some_and(|value| value.chars().count() > 128) {
            return Err("Custom status must be 128 characters or less".into());
        }
        if status_emoji.is_some_and(|value| value.chars().count() > 64) {
            return Err("Status emoji must be 64 characters or less".into());
        }

        let pool = self.db.as_ref().ok_or("Database unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|e| e.to_string())?;
        auth.validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        sqlx::query(
            "INSERT INTO user_presence (user_id,status,requested_status,custom_status,status_emoji,last_seen_at,updated_at) \
             VALUES (?,?,?,?,?,datetime('now'),datetime('now')) \
             ON CONFLICT(user_id) DO UPDATE SET status=excluded.status,requested_status=excluded.requested_status, \
             custom_status=excluded.custom_status,status_emoji=excluded.status_emoji,last_seen_at=datetime('now'),updated_at=datetime('now')",
        )
        .bind(&user_id)
        .bind(status)
        .bind(status)
        .bind(custom_status)
        .bind(status_emoji)
        .execute(&mut *transaction)
        .await
        .map_err(|e| format!("Failed to update presence: {e}"))?;
        transaction
            .commit()
            .await
            .map_err(|e| format!("Failed to update presence: {e}"))?;

        let _ = session.send(ChatEvent::OwnPresence {
            requested_status: status.to_owned(),
            effective_status: if status == "invisible" {
                "offline"
            } else {
                status
            }
            .to_owned(),
            custom_status: custom_status.map(str::to_owned),
            status_emoji: status_emoji.map(str::to_owned),
        });

        // Broadcast to all servers the user is a member of
        let server_ids: Vec<String> = self
            .servers
            .iter()
            .filter(|server| server.member_user_ids.contains(&user_id))
            .map(|server| server.id.clone())
            .collect();
        for server_id in server_ids {
            let identity = server_member_display_identity(pool, &server_id, &user_id)
                .await
                .map_err(|error| {
                    warn!(%error, %server_id, %user_id, "presence identity query failed");
                    "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
                })?;
            if let (Some((nickname, avatar_url)), Some(server)) =
                (identity, self.servers.get(&server_id))
            {
                let presence = super::events::PresenceInfo {
                    user_id: user_id.clone(),
                    nickname,
                    avatar_url,
                    status: if status == "invisible" {
                        "offline".into()
                    } else {
                        status.into()
                    },
                    custom_status: (status != "invisible")
                        .then(|| custom_status.map(str::to_owned))
                        .flatten(),
                    status_emoji: (status != "invisible")
                        .then(|| status_emoji.map(str::to_owned))
                        .flatten(),
                };
                let event = ChatEvent::PresenceUpdate {
                    server_id: server_id.clone(),
                    presence,
                };
                // Send to all sessions in this server's channels, deduplicated per server
                let mut notified = std::collections::HashSet::new();
                for channel_id in server.channel_ids.iter() {
                    if let Some(channel) = self.channels.get(channel_id) {
                        for &member_sid in &channel.members {
                            if member_sid != session_id
                                && notified.insert(member_sid)
                                && let Some(s) = self.sessions.get(&member_sid)
                            {
                                let _ = s.send_guarded(
                                    event.clone(),
                                    Some(super::user_session::DeliveryGuard::ServerMembership(
                                        vec![server_id.clone()],
                                    )),
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn send_own_presence(&self, session_id: ConnectionId) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let pool = self.db.as_ref().ok_or("Database unavailable")?;
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT requested_status,custom_status,status_emoji FROM user_presence WHERE user_id=?",
        )
        .bind(actor.user_id().as_str())
        .fetch_optional(pool)
        .await
        .map_err(|_| "Database unavailable".to_string())?;
        let (requested_status, custom_status, status_emoji) =
            row.unwrap_or_else(|| ("online".to_owned(), None, None));
        let effective_status = if requested_status == "invisible" {
            "offline".to_owned()
        } else {
            requested_status.clone()
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::OwnPresence {
                requested_status,
                effective_status,
                custom_status,
                status_emoji,
            });
        }
        Ok(())
    }

    /// Get presence list for all members of a server.
    pub async fn get_server_presences(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<Vec<super::events::PresenceInfo>, String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|_| "Not authenticated".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let mut tx = pool.begin().await.map_err(|error| {
            warn!(%error, "presence projection database begin failed");
            "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
        })?;
        super::authorization::AuthorizationService::new(pool.clone())
            .server_actor_permissions_in(&mut tx, auth, &actor, server_id)
            .await
            .map_err(|error| match error {
                super::authorization::AuthorizationError::Database(error) => {
                    warn!(%error, "presence projection authorization database failed");
                    "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
                }
                super::authorization::AuthorizationError::Authentication(_) => {
                    "UNAUTHENTICATED: authentication required".to_string()
                }
                super::authorization::AuthorizationError::Unavailable => {
                    "resource unavailable".to_string()
                }
            })?;
        let rows: Vec<PresenceProjectionRow> = sqlx::query_as(
            "SELECT sm.user_id AS user_id, \
                    COALESCE(NULLIF(sm.nickname,''),u.username) AS nickname, \
                    COALESCE(sm.avatar_url,u.avatar_url) AS avatar_url, \
                    p.requested_status AS requested_status, \
                    p.custom_status AS custom_status,p.status_emoji AS status_emoji \
             FROM server_members sm JOIN users u ON u.id=sm.user_id \
             LEFT JOIN user_presence p ON p.user_id=sm.user_id \
             WHERE sm.server_id=? AND NOT EXISTS( \
                 SELECT 1 FROM bans b WHERE b.server_id=sm.server_id AND b.user_id=sm.user_id \
             ) ORDER BY sm.user_id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| {
            warn!(%error, "presence projection database query failed");
            "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
        })?;
        tx.commit().await.map_err(|error| {
            warn!(%error, "presence projection database commit failed");
            "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
        })?;
        Ok(rows
            .into_iter()
            .map(
                |PresenceProjectionRow {
                     user_id,
                     nickname,
                     avatar_url,
                     requested_status,
                     custom_status,
                     status_emoji,
                 }| {
                    let live = self
                        .user_connections
                        .get(&user_id)
                        .is_some_and(|connections| !connections.is_empty());
                    let requested_status = requested_status.unwrap_or_else(|| "online".into());
                    let visible = live && requested_status != "invisible";
                    super::events::PresenceInfo {
                        user_id,
                        nickname,
                        avatar_url,
                        status: if visible {
                            requested_status
                        } else {
                            "offline".into()
                        },
                        custom_status: visible.then_some(custom_status).flatten(),
                        status_emoji: visible.then_some(status_emoji).flatten(),
                    }
                },
            )
            .collect())
    }

    // ── Server Nicknames ─────────────────────────────────────

    /// Set a user's server-specific display name.
    pub async fn set_server_nickname(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        nickname: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let base_display_name = self
            .get_session(session_id)
            .map(|session| session.nickname.clone())
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let user_id = actor.user_id().as_str().to_owned();
        let server_avatar_url = self
            .organization_service()?
            .set_server_nickname(&actor, &referenced_server_id(server_id)?, nickname)
            .await
            .map_err(String::from)?;

        // Broadcast nickname change
        let event = ChatEvent::ServerNicknameUpdate {
            server_id: server_id.to_string(),
            user_id: user_id.clone(),
            nickname: nickname.map(str::trim).map(str::to_owned),
            display_name: nickname
                .map(str::trim)
                .unwrap_or(&base_display_name)
                .to_owned(),
            server_avatar_url,
        };
        if let Some(server) = self.servers.get(server_id) {
            for channel_id in server.channel_ids.iter() {
                self.broadcast_to_channel(channel_id, &event, None);
            }
        }

        Ok(())
    }

    pub fn broadcast_server_member_identity(
        &self,
        server_id: &str,
        user_id: &str,
        nickname: Option<String>,
        display_name: String,
        avatar_url: Option<String>,
    ) {
        let event = ChatEvent::ServerNicknameUpdate {
            server_id: server_id.to_owned(),
            user_id: user_id.to_owned(),
            nickname,
            display_name,
            server_avatar_url: avatar_url,
        };
        if let Some(server) = self.servers.get(server_id) {
            for channel_id in &server.channel_ids {
                self.broadcast_to_channel(channel_id, &event, None);
            }
        }
    }

    // ── Search ───────────────────────────────────────────────

    /// Search messages in a server using full-text search.
    pub async fn search_messages(
        &self,
        actor: &crate::auth::authority::Actor,
        request: SearchMessagesRequest<'_>,
    ) -> Result<SearchResultsPage, SearchError> {
        let SearchMessagesRequest {
            server_id,
            query,
            channel_name,
            limit,
            offset,
            continuation,
        } = request;
        let pool = self.db.as_ref().ok_or_else(|| {
            SearchError::DependencyUnavailable(
                super::authorization::AuthorizationError::Unavailable,
            )
        })?;
        let plan = parse_search_query(query).map_err(SearchError::InvalidInput)?;
        if channel_name.is_some() && plan.channel.is_some() {
            return Err(SearchError::InvalidInput(
                "channel filter supplied twice".into(),
            ));
        }
        let effective_channel = channel_name.or(plan.channel.as_deref());

        // Resolve channel name to ID if provided (normalize for case-insensitive lookup)
        let channel_id = if let Some(ch_name) = effective_channel {
            let ch_name = normalize_channel_name(ch_name);
            Some(
                self.resolve_channel_id(server_id, &ch_name)
                    .map_err(SearchError::InvalidInput)?,
            )
        } else {
            None
        };

        let fingerprint = search_fingerprint(server_id, query, channel_id.as_deref());
        let decoded = continuation
            .map(|token| {
                let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
                validation.validate_exp = true;
                decode::<SearchContinuationClaims>(
                    token,
                    &DecodingKey::from_secret(self.search_token_secret.as_bytes()),
                    &validation,
                )
                .map(|data| data.claims)
                .map_err(|_| SearchError::InvalidContinuation)
            })
            .transpose()?;
        if decoded.as_ref().is_some_and(|claims| {
            claims.fingerprint != fingerprint
                || claims.credential_id != actor.credential_id().as_str()
        }) {
            return Err(SearchError::InvalidContinuation);
        }

        let auth = self.auth.get().ok_or_else(|| {
            SearchError::DependencyUnavailable(
                super::authorization::AuthorizationError::Unavailable,
            )
        })?;
        let authorization = super::authorization::AuthorizationService::new(pool.clone());
        let result_offset = decoded.as_ref().map_or(offset, |claims| claims.position);
        // Continuations are keyset cursors. Applying their logical display
        // position as a SQL OFFSET would skip a second page of rows.
        let query_offset = if decoded.is_some() { 0 } else { offset };
        let (mut rows, mut total, mut stamp) = authorization
            .search_messages(
                auth,
                actor,
                super::authorization::MessageSearch {
                    server_id,
                    query: plan.text.as_deref(),
                    requested_channel_id: channel_id.as_deref(),
                    sender: plan.sender.as_deref(),
                    has_attachment: plan.has_attachment,
                    has_link: plan.has_link,
                    before: plan.before.as_deref(),
                    after: plan.after.as_deref(),
                    after_inclusive: plan.after_inclusive,
                    limit,
                    offset: query_offset,
                    cursor_created_at: decoded
                        .as_ref()
                        .map(|claims| claims.before_created_at.as_str()),
                    cursor_message_id: decoded
                        .as_ref()
                        .map(|claims| claims.before_message_id.as_str()),
                },
            )
            .await
            .map_err(SearchError::from_authorization)?;
        let restarted = decoded
            .as_ref()
            .is_some_and(|claims| claims.authorization_version != stamp.server_version);
        let result_offset = if restarted { 0 } else { result_offset };
        if restarted {
            (rows, total, stamp) = authorization
                .search_messages(
                    auth,
                    actor,
                    super::authorization::MessageSearch {
                        server_id,
                        query: plan.text.as_deref(),
                        requested_channel_id: channel_id.as_deref(),
                        sender: plan.sender.as_deref(),
                        has_attachment: plan.has_attachment,
                        has_link: plan.has_link,
                        before: plan.before.as_deref(),
                        after: plan.after.as_deref(),
                        after_inclusive: plan.after_inclusive,
                        limit,
                        offset: 0,
                        cursor_created_at: None,
                        cursor_message_id: None,
                    },
                )
                .await
                .map_err(SearchError::from_authorization)?;
        }

        let next_continuation = if result_offset + rows.len() as i64 >= total {
            None
        } else {
            let last = rows.last().ok_or(SearchError::InvalidContinuation)?;
            let claims = SearchContinuationClaims {
                exp: (Utc::now() + chrono::Duration::minutes(15)).timestamp(),
                credential_id: actor.credential_id().as_str().to_owned(),
                fingerprint,
                authorization_version: stamp.server_version,
                before_created_at: last.created_at.clone(),
                before_message_id: last.id.clone(),
                position: result_offset + rows.len() as i64,
            };
            Some(
                encode(
                    &Header::default(),
                    &claims,
                    &EncodingKey::from_secret(self.search_token_secret.as_bytes()),
                )
                .map_err(|_| {
                    SearchError::DependencyUnavailable(
                        super::authorization::AuthorizationError::Unavailable,
                    )
                })?,
            )
        };

        let mut channel_names = std::collections::HashMap::new();
        for channel_id in rows.iter().filter_map(|row| row.channel_id.as_deref()) {
            if channel_names.contains_key(channel_id) {
                continue;
            }
            let name: String =
                sqlx::query_scalar("SELECT name FROM channels WHERE id=? AND server_id=?")
                    .bind(channel_id)
                    .bind(server_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|error| {
                        SearchError::DependencyUnavailable(
                            super::authorization::AuthorizationError::Database(error),
                        )
                    })?
                    .ok_or(SearchError::ResourceUnavailable)?;
            channel_names.insert(channel_id.to_owned(), name);
        }

        let results = rows
            .drain(..)
            .map(|row| {
                let channel_id = row.channel_id.unwrap_or_default();
                let channel_name = channel_names.get(&channel_id).cloned().unwrap_or_default();
                super::events::SearchResultMessage {
                    id: row.id,
                    from: row.sender_nick,
                    content: row.content,
                    timestamp: row.created_at,
                    channel_id,
                    channel_name,
                    edited_at: row.edited_at,
                }
            })
            .collect();

        Ok(SearchResultsPage {
            results,
            total_count: total,
            offset: result_offset,
            next_continuation,
            restarted,
            stamp,
        })
    }

    pub async fn authorization_stamp_is_current(
        &self,
        actor: &crate::auth::authority::Actor,
        stamp: &super::authorization::AuthorizationStamp,
    ) -> bool {
        let Some(pool) = &self.db else {
            return false;
        };
        let Some(auth) = self.auth.get() else {
            return false;
        };
        if auth.validate_actor(actor).await.is_err() {
            return false;
        }
        super::authorization::AuthorizationService::new(pool.clone())
            .stamp_is_current(stamp)
            .await
            .unwrap_or(false)
    }

    /// Revalidate queued authorization evidence at the final transport boundary.
    pub async fn delivery_guard_is_current(
        &self,
        actor: &crate::auth::authority::Actor,
        guard: &crate::engine::user_session::DeliveryGuard,
    ) -> bool {
        use crate::engine::authorization::ConversationAction;
        use crate::engine::user_session::DeliveryGuard;

        match guard {
            DeliveryGuard::ActorCurrent => self.actor_is_current(actor).await,
            DeliveryGuard::Stamps(stamps) => {
                for stamp in stamps {
                    if !self.authorization_stamp_is_current(actor, stamp).await {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::Conversations(conversation_ids) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = super::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for conversation_id in conversation_ids {
                    if service
                        .authorize_conversation_actor_in(
                            &mut connection,
                            auth,
                            actor,
                            conversation_id,
                            ConversationAction::Read,
                        )
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::Channels(channel_ids) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = super::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for channel_id in channel_ids {
                    if service
                        .authorize_actor_in(
                            &mut connection,
                            auth,
                            actor,
                            channel_id,
                            super::authorization::ChannelAction::View,
                        )
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::ChannelActions(requirements) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = super::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for (channel_id, action) in requirements {
                    if service
                        .authorize_actor_in(&mut connection, auth, actor, channel_id, *action)
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::ServerMembership(server_ids) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                if auth
                    .validate_actor_in(&mut connection, actor)
                    .await
                    .is_err()
                {
                    return false;
                }
                for server_id in server_ids {
                    let current: Result<bool, _> = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM server_members sm \
                         WHERE sm.server_id=? AND sm.user_id=? \
                         AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=sm.server_id AND b.user_id=sm.user_id))",
                    )
                    .bind(server_id)
                    .bind(actor.user_id().as_str())
                    .fetch_one(&mut *connection)
                    .await;
                    if !matches!(current, Ok(true)) {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::ServerPermissions(requirements) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = super::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for (server_id, permissions) in requirements {
                    if service
                        .require_server_actor_in(
                            &mut connection,
                            auth,
                            actor,
                            server_id,
                            *permissions,
                        )
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::BotInstallationScopes(requirements) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = super::authorization::AuthorizationService::new(pool.clone());
                for (server_id, scope) in requirements {
                    if service
                        .authorize_bot_installation_scope(auth, actor, server_id, scope)
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
        }
    }

    pub async fn actor_is_current(&self, actor: &crate::auth::authority::Actor) -> bool {
        let Some(auth) = self.auth.get() else {
            return false;
        };
        auth.validate_actor(actor).await.is_ok()
    }

    // ── Notification Settings ────────────────────────────────

    /// Update notification settings for a user in a server or channel.
    pub async fn update_notification_settings(
        &self,
        session_id: ConnectionId,
        params: &UpdateNotificationSettingsParams<'_>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;

        match params.level {
            "all" | "mentions" | "none" | "default" => {}
            _ => return Err("Invalid level. Must be: all, mentions, none, default".into()),
        }

        let pool = self.db.as_ref().ok_or("No database configured")?;
        let id = Uuid::new_v4().to_string();
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        let authorization = super::authorization::AuthorizationService::new(pool.clone());
        if let Some(channel_id) = params.channel_id {
            authorization
                .authorize_actor_in(
                    &mut transaction,
                    self.auth.get().ok_or("Authentication unavailable")?,
                    &actor,
                    channel_id,
                    super::authorization::ChannelAction::View,
                )
                .await
                .map_err(|_| "resource unavailable".to_string())?;
            let actual_server: Option<String> =
                sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                    .bind(channel_id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(|_| "resource unavailable".to_string())?;
            if actual_server.as_deref() != Some(params.server_id) {
                return Err("resource unavailable".into());
            }
        } else {
            authorization
                .require_server_actor_in(
                    &mut transaction,
                    self.auth.get().ok_or("Authentication unavailable")?,
                    &actor,
                    params.server_id,
                    Permissions::VIEW_CHANNELS,
                )
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        }
        let conflict = if params.channel_id.is_some() {
            " ON CONFLICT(user_id,channel_id) WHERE channel_id IS NOT NULL DO UPDATE SET "
        } else {
            " ON CONFLICT(user_id,server_id) WHERE server_id IS NOT NULL AND channel_id IS NULL DO UPDATE SET "
        };
        let sql = format!(
            "INSERT INTO notification_settings(id,user_id,server_id,channel_id,level, \
             suppress_everyone,suppress_roles,muted,mute_until,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,datetime('now')){conflict} \
             level=excluded.level,suppress_everyone=excluded.suppress_everyone, \
             suppress_roles=excluded.suppress_roles,muted=excluded.muted, \
             mute_until=excluded.mute_until,updated_at=datetime('now')"
        );
        // Only the literal conflict clause above is interpolated; all values are bound.
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&id)
            .bind(actor.user_id().as_str())
            .bind(params.server_id)
            .bind(params.channel_id)
            .bind(params.level)
            .bind(params.suppress_everyone as i32)
            .bind(params.suppress_roles as i32)
            .bind(params.muted as i32)
            .bind(params.mute_until)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        Ok(())
    }

    /// Get notification settings for a user in a server.
    pub async fn get_notification_settings(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<Vec<super::events::NotificationSettingInfo>, String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.clone().ok_or("Not authenticated")?;

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows =
            crate::db::queries::notifications::get_notification_settings(pool, &user_id, server_id)
                .await
                .map_err(|e| format!("Failed to get notification settings: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|r| super::events::NotificationSettingInfo {
                id: r.id,
                server_id: r.server_id,
                channel_id: r.channel_id,
                level: r.level,
                suppress_everyone: r.suppress_everyone != 0,
                suppress_roles: r.suppress_roles != 0,
                muted: r.muted != 0,
                mute_until: r.mute_until,
            })
            .collect())
    }

    // ── Pinning ─────────────────────────────────────────────────

    /// Pin a message in a channel. Requires MANAGE_MESSAGES permission.
    pub async fn pin_message(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                &channel_id,
                super::authorization::ChannelAction::ManageMessages,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let msg = sqlx::query(
            "SELECT sender_nick,content,created_at FROM messages \
             WHERE id=? AND channel_id=? AND server_id=? AND deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(&channel_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?
        .ok_or_else(|| "resource unavailable".to_string())?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM pinned_messages WHERE channel_id=? AND message_id=?",
        )
        .bind(&channel_id)
        .bind(message_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        if existing.is_some() {
            transaction
                .commit()
                .await
                .map_err(|_| "resource unavailable".to_string())?;
            return Ok(());
        }
        let pin_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pinned_messages WHERE channel_id=?")
                .bind(&channel_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        if pin_count >= 50 {
            return Err("Channel has reached the maximum of 50 pinned messages".into());
        }
        let pin_id = Uuid::new_v4().to_string();
        let pinned_at: String = sqlx::query_scalar(
            "INSERT INTO pinned_messages(id,channel_id,message_id,pinned_by) VALUES(?,?,?,?) \
             RETURNING pinned_at",
        )
        .bind(&pin_id)
        .bind(&channel_id)
        .bind(message_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let pin = PinnedMessageInfo {
            id: pin_id,
            message_id: message_id.to_string(),
            channel_id: channel_id.clone(),
            pinned_by: actor.user_id().as_str().to_owned(),
            pinned_at,
            from: msg.get(0),
            content: msg.get(1),
            timestamp: msg.get(2),
        };

        let event = ChatEvent::MessagePin {
            server_id: server_id.to_string(),
            channel: channel_name,
            pin,
        };
        if let Some(channel) = self.channels.get(&channel_id) {
            for member_id in &channel.members {
                if let Some(session) = self.sessions.get(member_id) {
                    let _ = session.send_guarded(
                        event.clone(),
                        Some(super::user_session::DeliveryGuard::ChannelActions(vec![(
                            channel_id.clone(),
                            super::authorization::ChannelAction::ReadHistory,
                        )])),
                    );
                }
            }
        }

        Ok(())
    }

    /// Unpin a message from a channel. Requires MANAGE_MESSAGES permission.
    pub async fn unpin_message(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                &channel_id,
                super::authorization::ChannelAction::ManageMessages,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let removed = sqlx::query(
            "DELETE FROM pinned_messages WHERE channel_id=? AND message_id=? \
             AND EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)",
        )
        .bind(&channel_id)
        .bind(message_id)
        .bind(&channel_id)
        .bind(server_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?
        .rows_affected();
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        if removed == 0 {
            return Ok(());
        }

        let event = ChatEvent::MessageUnpin {
            server_id: server_id.to_string(),
            channel: channel_name,
            message_id: message_id.to_string(),
        };
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }

    /// Get all pinned messages in a channel. Sends PinnedMessages event to the requesting session.
    pub async fn get_pinned_messages(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                &channel_id,
                super::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        let pin_rows = sqlx::query(
            "SELECT p.id,p.message_id,p.channel_id,p.pinned_by,p.pinned_at, \
                    m.sender_nick,m.content,m.created_at,m.deleted_at \
             FROM pinned_messages p JOIN messages m ON m.id=p.message_id \
             WHERE p.channel_id=? ORDER BY p.pinned_at DESC,p.id DESC",
        )
        .bind(&channel_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;

        let mut pins = Vec::new();
        for row in pin_rows {
            let deleted = row.get::<Option<String>, _>(8).is_some();
            pins.push(PinnedMessageInfo {
                id: row.get(0),
                message_id: row.get(1),
                channel_id: row.get(2),
                pinned_by: row.get(3),
                pinned_at: row.get(4),
                from: if deleted {
                    "unknown".to_owned()
                } else {
                    row.get(5)
                },
                content: if deleted {
                    "[deleted]".to_owned()
                } else {
                    row.get(6)
                },
                timestamp: if deleted { String::new() } else { row.get(7) },
            });
        }
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        let _ = session.send_guarded(
            ChatEvent::PinnedMessages {
                server_id: server_id.to_string(),
                channel: channel_name,
                pins,
            },
            Some(super::user_session::DeliveryGuard::ChannelActions(vec![(
                channel_id,
                super::authorization::ChannelAction::ReadHistory,
            )])),
        );

        Ok(())
    }

    // ── Threads ─────────────────────────────────────────────────

    /// Create a thread from a message in a channel.
    pub async fn create_thread(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        parent_channel_name: &str,
        name: &str,
        message_id: &str,
        is_private: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;

        let parent_channel_name = normalize_channel_name(parent_channel_name);
        let parent_channel_id = self.resolve_channel_id(server_id, &parent_channel_name)?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        super::authorization::AuthorizationService::new(
            self.db.clone().ok_or("No database configured")?,
        )
        .authorize_actor_in(
            &mut transaction,
            self.auth.get().ok_or("Authentication unavailable")?,
            &actor,
            &parent_channel_id,
            super::authorization::ChannelAction::Send,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;

        // Validate thread name
        if name.is_empty() || name.len() > 100 {
            return Err("Thread name must be between 1 and 100 characters".into());
        }

        let channel_type = if is_private {
            "private_thread"
        } else {
            "public_thread"
        };

        let thread_id = Uuid::new_v4().to_string();
        let thread_name = normalize_channel_name(name);

        // Check name uniqueness within server
        if self
            .channel_name_index
            .contains_key(&(server_id.to_string(), thread_name.clone()))
        {
            return Err(format!(
                "A channel or thread named {thread_name} already exists"
            ));
        }

        crate::db::queries::threads::create_thread_in(
            &mut transaction,
            &crate::db::queries::threads::CreateThreadParams {
                channel_id: &thread_id,
                server_id,
                name: &thread_name,
                channel_type,
                parent_message_id: message_id,
                parent_channel_id: &parent_channel_id,
                creator_user_id: actor.user_id().as_str(),
                auto_archive_minutes: 1440,
            },
        )
        .await
        .map_err(|e| format!("Failed to create thread: {e}"))?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;

        // Add to in-memory state
        let mut ch = ChannelState::new(
            thread_id.clone(),
            server_id.to_string(),
            thread_name.clone(),
        );
        ch.channel_type = channel_type.to_string();
        ch.thread_parent_message_id = Some(message_id.to_string());
        ch.thread_creator_user_id = Some(actor.user_id().as_str().to_string());
        ch.auto_archive_minutes = 1440;
        ch.is_private = is_private;

        self.channel_name_index.insert(
            (server_id.to_string(), thread_name.clone()),
            thread_id.clone(),
        );
        if let Some(mut srv) = self.servers.get_mut(server_id) {
            srv.channel_ids.insert(thread_id.clone());
        }
        self.channels.insert(thread_id.clone(), ch);

        let thread_info = ThreadInfo {
            id: thread_id.clone(),
            name: thread_name,
            channel_type: channel_type.to_string(),
            parent_message_id: Some(message_id.to_string()),
            creator_user_id: Some(actor.user_id().as_str().to_string()),
            archived: false,
            state_version: 1,
            tags_version: 1,
            tag_ids: Vec::new(),
            auto_archive_minutes: 1440,
            message_count: 0,
            created_at: Utc::now().to_rfc3339(),
        };

        let event = ChatEvent::ThreadCreate {
            server_id: server_id.to_string(),
            parent_channel: parent_channel_name,
            thread: thread_info,
        };
        if is_private {
            if let Some(connections) = self.user_connections.get(actor.user_id().as_str()) {
                for session_id in connections.iter() {
                    if let Some(session) = self.sessions.get(session_id) {
                        let _ = session.send_guarded(
                            event.clone(),
                            Some(super::user_session::DeliveryGuard::Channels(vec![
                                thread_id.clone(),
                            ])),
                        );
                    }
                }
            }
        } else {
            self.broadcast_to_channel(&parent_channel_id, &event, None);
        }

        Ok(())
    }

    /// Archive a thread. Requires MANAGE_CHANNELS permission.
    pub async fn archive_thread(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        self.set_thread_archived(session_id, server_id, thread_id, true)
            .await
    }

    pub async fn unarchive_thread(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        self.set_thread_archived(session_id, server_id, thread_id, false)
            .await
    }

    async fn set_thread_archived(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
        archived: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        super::authorization::AuthorizationService::new(
            self.db.clone().ok_or("No database configured")?,
        )
        .authorize_actor_in(
            &mut transaction,
            self.auth.get().ok_or("Authentication unavailable")?,
            &actor,
            thread_id,
            super::authorization::ChannelAction::Manage,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let actual_server: Option<String> =
            sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                .bind(thread_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
        if actual_server.as_deref() != Some(server_id) {
            return Err("resource unavailable".into());
        }
        let version = crate::db::queries::threads::set_thread_archived_in(
            &mut transaction,
            thread_id,
            archived,
        )
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        Self::insert_thread_state_event_in(
            &mut transaction,
            thread_id,
            version,
            archived,
            archived.then_some("manual"),
            actor.user_id().as_str(),
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        self.project_thread_state(thread_id).await
    }

    /// List threads for a channel. Sends ThreadList event to the requesting session.
    pub async fn list_threads(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let rows = super::authorization::AuthorizationService::new(pool.clone())
            .visible_channels(user_id, server_id)
            .await
            .map_err(|_| "resource unavailable".to_string())?
            .into_iter()
            .filter(|row| row.parent_channel_id.as_deref() == Some(channel_id.as_str()))
            .collect::<Vec<_>>();

        let mut guarded_channels = vec![channel_id];
        guarded_channels.extend(rows.iter().map(|row| row.id.clone()));
        let mut threads = Vec::with_capacity(rows.len());
        for row in rows {
            let tag_ids = sqlx::query_scalar(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(&row.id)
            .fetch_all(pool)
            .await
            .map_err(|error| format!("Failed to list thread tags: {error}"))?;
            threads.push(ThreadInfo {
                id: row.id,
                name: row.name,
                channel_type: row.channel_type,
                parent_message_id: row.thread_parent_message_id,
                creator_user_id: row.thread_creator_user_id,
                archived: row.archived != 0,
                state_version: row.thread_state_version,
                tags_version: row.thread_tags_version,
                tag_ids,
                auto_archive_minutes: row.thread_auto_archive_minutes,
                message_count: 0, // would need a count query; returning 0 for now
                created_at: row.created_at,
            });
        }

        let _ = session.send_guarded(
            ChatEvent::ThreadList {
                server_id: server_id.to_string(),
                channel: channel_name,
                threads,
            },
            Some(super::user_session::DeliveryGuard::Channels(
                guarded_channels,
            )),
        );

        Ok(())
    }

    pub async fn create_forum_tag(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        name: &str,
        emoji: Option<&str>,
        moderated: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = super::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let tag = service
            .create_tag(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                super::forum::CreateForumTag {
                    server_id,
                    channel_id: &channel_id,
                    name,
                    emoji,
                    moderated,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;
        let event = ChatEvent::ForumTagUpdate {
            server_id: server_id.to_string(),
            channel: channel_name,
            tag,
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                event.clone(),
                Some(super::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }
        self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_forum_tag(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        tag_id: &str,
        name: &str,
        emoji: Option<&str>,
        moderated: bool,
        position: i32,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = super::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let tag = service
            .update_tag(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                super::forum::UpdateForumTag {
                    server_id,
                    channel_id: &channel_id,
                    tag_id,
                    name,
                    emoji,
                    moderated,
                    position,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;
        let event = ChatEvent::ForumTagUpdate {
            server_id: server_id.to_string(),
            channel: channel_name,
            tag,
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                event.clone(),
                Some(super::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }
        self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        Ok(())
    }

    pub async fn delete_forum_tag(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = super::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let mutations = service
            .delete_tag(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                &channel_id,
                tag_id,
            )
            .await
            .map_err(|error| error.wire_message())?;
        for mutation in mutations {
            if let Some(mut thread) = self.channels.get_mut(&mutation.thread_id)
                && mutation.version >= thread.thread_tags_version
            {
                thread.thread_tags_version = mutation.version;
                thread.thread_tag_ids = mutation.tag_ids.clone();
            }
            let event = ChatEvent::ThreadTagUpdate {
                server_id: server_id.to_string(),
                thread_id: mutation.thread_id.clone(),
                version: mutation.version,
                tag_ids: mutation.tag_ids,
            };
            if let Some(session) = self.get_session(session_id) {
                let _ = session.send_guarded(
                    event.clone(),
                    Some(super::user_session::DeliveryGuard::Channels(vec![
                        mutation.thread_id.clone(),
                    ])),
                );
            }
            self.broadcast_to_channel(&mutation.thread_id, &event, Some(session_id));
        }
        let event = ChatEvent::ForumTagDelete {
            server_id: server_id.to_string(),
            channel: channel_name,
            tag_id: tag_id.to_string(),
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                event.clone(),
                Some(super::user_session::DeliveryGuard::Channels(vec![
                    channel_id.clone(),
                ])),
            );
        }
        self.broadcast_to_channel(&channel_id, &event, Some(session_id));
        Ok(())
    }

    pub async fn list_forum_tags(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let session = self.get_session(session_id).ok_or("resource unavailable")?;
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let service = super::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let tags = service
            .list_tags(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                &channel_id,
            )
            .await
            .map_err(|error| error.wire_message())?;
        let _ = session.send_guarded(
            ChatEvent::ForumTagList {
                server_id: server_id.to_string(),
                channel: channel_name,
                tags,
            },
            Some(super::user_session::DeliveryGuard::Channels(vec![
                channel_id,
            ])),
        );
        Ok(())
    }

    pub async fn set_thread_tags(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
        tag_ids: Vec<String>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let service = super::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let mutation = service
            .set_thread_tags(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                thread_id,
                tag_ids,
            )
            .await
            .map_err(|error| error.wire_message())?;
        if let Some(mut channel) = self.channels.get_mut(thread_id)
            && mutation.version >= channel.thread_tags_version
        {
            channel.thread_tags_version = mutation.version;
            channel.thread_tag_ids = mutation.tag_ids.clone();
        }
        self.broadcast_to_channel(
            thread_id,
            &ChatEvent::ThreadTagUpdate {
                server_id: server_id.to_string(),
                thread_id: thread_id.to_string(),
                version: mutation.version,
                tag_ids: mutation.tag_ids,
            },
            None,
        );
        Ok(())
    }

    pub async fn get_thread_tags(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let session = self.get_session(session_id).ok_or("resource unavailable")?;
        let service = super::forum::ForumService::new(
            self.db.clone().ok_or("No database configured")?,
            self.write_admission
                .clone()
                .ok_or("Write admission unavailable")?,
        );
        let (version, tag_ids) = service
            .get_thread_tags(
                self.auth.get().ok_or("Authentication unavailable")?,
                &actor,
                server_id,
                thread_id,
            )
            .await
            .map_err(|error| error.wire_message())?;
        let _ = session.send_guarded(
            ChatEvent::ThreadTagUpdate {
                server_id: server_id.to_string(),
                thread_id: thread_id.to_string(),
                version,
                tag_ids,
            },
            Some(super::user_session::DeliveryGuard::Channels(vec![
                thread_id.to_string(),
            ])),
        );
        Ok(())
    }

    // ── Bookmarks    // ── Bookmarks ───────────────────────────────────────────────

    /// Add a bookmark on a message for the authenticated user.
    pub async fn add_bookmark(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        let msg = sqlx::query(
            "SELECT channel_id,sender_nick,content,created_at FROM messages \
             WHERE id=? AND channel_id IS NOT NULL AND deleted_at IS NULL",
        )
        .bind(message_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?
        .ok_or_else(|| "resource unavailable".to_string())?;
        let channel_id: String = msg.get(0);
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                &actor,
                &channel_id,
                super::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let bookmark_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO bookmarks(id,user_id,message_id,note) VALUES(?,?,?,?) \
             ON CONFLICT(user_id,message_id) DO UPDATE SET note=excluded.note",
        )
        .bind(&bookmark_id)
        .bind(actor.user_id().as_str())
        .bind(message_id)
        .bind(note)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let stored = sqlx::query(
            "SELECT id,created_at,note FROM bookmarks WHERE user_id=? AND message_id=?",
        )
        .bind(actor.user_id().as_str())
        .bind(message_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let bookmark = BookmarkInfo {
            id: stored.get(0),
            message_id: message_id.to_string(),
            channel_id: channel_id.clone(),
            from: msg.get(1),
            content: msg.get(2),
            timestamp: msg.get(3),
            note: stored.get(2),
            created_at: stored.get(1),
        };
        let _ = session.send_guarded(
            ChatEvent::BookmarkAdd { bookmark },
            Some(crate::engine::user_session::DeliveryGuard::ChannelActions(
                vec![(channel_id, super::authorization::ChannelAction::ReadHistory)],
            )),
        );

        Ok(())
    }

    /// Remove a bookmark for the authenticated user.
    pub async fn remove_bookmark(
        &self,
        session_id: ConnectionId,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        auth.validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        sqlx::query("DELETE FROM bookmarks WHERE user_id=? AND message_id=?")
            .bind(actor.user_id().as_str())
            .bind(message_id)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let _ = session.send_guarded(
            ChatEvent::BookmarkRemove {
                message_id: message_id.to_string(),
            },
            Some(crate::engine::user_session::DeliveryGuard::ActorCurrent),
        );

        Ok(())
    }

    /// List all bookmarks for the authenticated user. Sends BookmarkList event to the session.
    pub async fn list_bookmarks(&self, session_id: ConnectionId) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        auth.validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let rows = sqlx::query(
            "SELECT b.id,b.message_id,b.note,b.created_at,m.channel_id,m.sender_nick, \
                    m.content,m.created_at,m.deleted_at \
             FROM bookmarks b JOIN messages m ON m.id=b.message_id \
             WHERE b.user_id=? ORDER BY b.created_at DESC,b.id DESC",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable".to_string())?;
        let mut bookmarks = Vec::new();
        let authorization = super::authorization::AuthorizationService::new(pool.clone());
        let mut guarded_channels = Vec::new();
        for row in rows {
            let channel_id: String = row.get(4);
            if authorization
                .authorize_actor_in(
                    &mut transaction,
                    auth,
                    &actor,
                    &channel_id,
                    super::authorization::ChannelAction::ReadHistory,
                )
                .await
                .is_err()
            {
                continue;
            }
            guarded_channels.push((
                channel_id.clone(),
                super::authorization::ChannelAction::ReadHistory,
            ));
            let deleted = row.get::<Option<String>, _>(8).is_some();
            bookmarks.push(BookmarkInfo {
                id: row.get(0),
                message_id: row.get(1),
                channel_id,
                from: if deleted {
                    "unknown".to_owned()
                } else {
                    row.get(5)
                },
                content: if deleted {
                    "[deleted]".to_owned()
                } else {
                    row.get(6)
                },
                timestamp: if deleted { String::new() } else { row.get(7) },
                note: row.get(2),
                created_at: row.get(3),
            });
        }
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        let _ = session.send_guarded(
            ChatEvent::BookmarkList { bookmarks },
            Some(crate::engine::user_session::DeliveryGuard::ChannelActions(
                guarded_channels,
            )),
        );

        Ok(())
    }

    // ── Phase 6: Moderation ─────────────────────────────────────

    /// Broadcast a ChatEvent to all connected sessions that belong to a server.
    pub fn broadcast_to_server(&self, server_id: &str, event: &ChatEvent) {
        let Some(server) = self.servers.get(server_id) else {
            return;
        };
        let member_ids: Vec<String> = server.member_user_ids.iter().cloned().collect();
        drop(server);

        for session in self.sessions.iter() {
            if let Some(uid) = &session.user_id
                && member_ids.contains(uid)
            {
                let _ = session.send(event.clone());
            }
        }
    }

    /// Kick a member from a server.
    pub async fn kick_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(), String> {
        self.kick_member_in_channel(session_id, server_id, target_user_id, reason, None)
            .await
    }

    /// Kick a member with channel-scoped permission check.
    /// When `channel_id` is Some, the permission check considers channel overrides,
    /// allowing moderators with per-channel KICK_MEMBERS to kick from that channel.
    pub async fn kick_member_in_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
        channel_id: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let server_resource_id = referenced_server_id(server_id)?;
        let channel_resource_id = channel_id.map(referenced_channel_id).transpose()?;
        self.moderation_service()?
            .kick_member(
                &actor,
                &server_resource_id,
                target_user_id,
                reason,
                channel_resource_id.as_ref(),
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Remove from in-memory server state
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(target_user_id);
        }
        self.evict_user_from_server_subscriptions(server_id, target_user_id);

        // Broadcast kick event to server members
        let event = ChatEvent::MemberKick {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            kicked_by: actor.user_id().as_str().to_owned(),
            reason: reason.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Ban a member from a server, optionally deleting their messages.
    pub async fn ban_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        reason: Option<&str>,
        delete_message_days: i32,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .ban_member(
                &actor,
                &referenced_server_id(server_id)?,
                target_user_id,
                reason,
                delete_message_days,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Remove from in-memory server state
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.member_user_ids.remove(target_user_id);
        }
        self.evict_user_from_server_subscriptions(server_id, target_user_id);

        // Broadcast
        let event = ChatEvent::MemberBan {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            banned_by: actor.user_id().as_str().to_owned(),
            reason: reason.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Unban a member from a server.
    pub async fn unban_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .unban_member(&actor, &referenced_server_id(server_id)?, target_user_id)
            .await
            .map_err(|error| error.wire_message())?;

        // Broadcast
        let event = ChatEvent::MemberUnban {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Get the list of bans for a server.
    pub async fn list_bans(&self, session_id: ConnectionId, server_id: &str) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rows = self
            .moderation_service()?
            .list_bans(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(|error| error.wire_message())?;

        let bans: Vec<BanInfo> = rows
            .into_iter()
            .map(|r| BanInfo {
                id: r.id,
                user_id: r.user_id,
                banned_by: r.banned_by,
                reason: r.reason,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BanList {
                server_id: server_id.to_string(),
                bans,
            });
        }

        Ok(())
    }

    /// Set a timeout on a member (or clear it).
    pub async fn timeout_member(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        target_user_id: &str,
        timeout_until: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .timeout_member(
                &actor,
                &referenced_server_id(server_id)?,
                target_user_id,
                timeout_until,
                reason,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Broadcast
        let event = ChatEvent::MemberTimeout {
            server_id: server_id.to_string(),
            user_id: target_user_id.to_string(),
            timeout_until: timeout_until.map(String::from),
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Set slow mode on a channel.
    pub async fn set_slowmode(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        seconds: i32,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|v| v.clone())
            .ok_or_else(moderation_unavailable)?;
        self.moderation_service()?
            .set_slowmode(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                seconds,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Update in-memory state
        if let Some(mut ch) = self.channels.get_mut(&channel_id) {
            ch.slowmode_seconds = seconds;
        }

        // Broadcast
        let event = ChatEvent::SlowModeUpdate {
            server_id: server_id.to_string(),
            channel: channel_name.to_string(),
            seconds,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Set NSFW flag on a channel.
    pub async fn set_nsfw(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        is_nsfw: bool,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let channel_id = self
            .channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|v| v.clone())
            .ok_or_else(moderation_unavailable)?;
        self.moderation_service()?
            .set_nsfw(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                is_nsfw,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Update in-memory state
        if let Some(mut ch) = self.channels.get_mut(&channel_id) {
            ch.is_nsfw = is_nsfw;
        }

        // Broadcast
        let event = ChatEvent::NsfwUpdate {
            server_id: server_id.to_string(),
            channel: channel_name.to_string(),
            is_nsfw,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Bulk delete messages in a channel (up to 100).
    pub async fn bulk_delete_messages(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_ids: Vec<String>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let channel_id =
            self.resolve_channel_id(server_id, &normalize_channel_name(channel_name))?;
        self.moderation_service()?
            .bulk_delete_messages(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                &message_ids,
            )
            .await
            .map_err(|error| error.wire_message())?;

        // Broadcast
        let event = ChatEvent::BulkMessageDelete {
            server_id: server_id.to_string(),
            channel: channel_name.to_string(),
            message_ids,
        };
        self.broadcast_to_server(server_id, &event);

        Ok(())
    }

    /// Get audit log entries for a server.
    pub async fn get_audit_log(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        action_type: Option<&str>,
        limit: i64,
        before: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rows = self
            .moderation_service()?
            .list_audit_log(
                &actor,
                &referenced_server_id(server_id)?,
                action_type,
                limit,
                before,
            )
            .await
            .map_err(|error| error.wire_message())?;

        let entries: Vec<AuditLogEntry> = rows
            .into_iter()
            .map(|r| AuditLogEntry {
                id: r.id,
                actor_id: r.actor_id,
                actor_username_snapshot: r.actor_username_snapshot,
                actor_avatar_snapshot: r.actor_avatar_snapshot,
                action_type: r.action_type,
                target_type: r.target_type,
                target_id: r.target_id,
                reason: r.reason,
                changes: r.changes,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AuditLogEntries {
                server_id: server_id.to_string(),
                entries,
            });
        }

        Ok(())
    }

    // ── AutoMod ──

    /// Create an automod rule.
    pub async fn create_automod_rule(
        &self,
        session_id: ConnectionId,
        params: &CreateAutomodRuleRequest<'_>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let server_id = params.server_id;
        let name = params.name;
        let rule_type = params.rule_type;
        let config = params.config;
        let action_type = params.action_type;
        let timeout_duration_seconds = params.timeout_duration_seconds;
        let rule_id = self
            .moderation_service()?
            .create_automod_rule(
                &actor,
                &super::moderation::CreateAutomodRule {
                    server_id: &referenced_server_id(server_id)?,
                    name,
                    rule_type,
                    config,
                    action_type,
                    timeout_duration_seconds,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;

        let rule = AutomodRuleInfo {
            id: rule_id,
            name: name.to_string(),
            enabled: true,
            rule_type: rule_type.to_string(),
            config: config.to_string(),
            action_type: action_type.to_string(),
            timeout_duration_seconds,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleUpdate {
                server_id: server_id.to_string(),
                rule,
            });
        }

        Ok(())
    }

    /// Update an automod rule.
    pub async fn update_automod_rule(
        &self,
        session_id: ConnectionId,
        params: &UpdateAutomodRuleRequest<'_>,
    ) -> Result<(), String> {
        let server_id = params.server_id;
        let rule_id = params.rule_id;
        let name = params.name;
        let enabled = params.enabled;
        let config = params.config;
        let action_type = params.action_type;
        let timeout_duration_seconds = params.timeout_duration_seconds;

        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rule_type = self
            .moderation_service()?
            .update_automod_rule(
                &actor,
                &super::moderation::UpdateAutomodRule {
                    server_id: &referenced_server_id(server_id)?,
                    rule_id,
                    name,
                    enabled,
                    config,
                    action_type,
                    timeout_duration_seconds,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;

        let rule = AutomodRuleInfo {
            id: rule_id.to_string(),
            name: name.to_string(),
            enabled,
            rule_type,
            config: config.to_string(),
            action_type: action_type.to_string(),
            timeout_duration_seconds,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleUpdate {
                server_id: server_id.to_string(),
                rule,
            });
        }

        Ok(())
    }

    /// Delete an automod rule.
    pub async fn delete_automod_rule(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        rule_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .delete_automod_rule(&actor, &referenced_server_id(server_id)?, rule_id)
            .await
            .map_err(|error| error.wire_message())?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleDelete {
                server_id: server_id.to_string(),
                rule_id: rule_id.to_string(),
            });
        }

        Ok(())
    }

    /// List automod rules for a server.
    pub async fn list_automod_rules(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rows = self
            .moderation_service()?
            .list_automod_rules(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(|error| error.wire_message())?;

        let rules: Vec<AutomodRuleInfo> = rows
            .into_iter()
            .map(|r| AutomodRuleInfo {
                id: r.id,
                name: r.name,
                enabled: r.enabled != 0,
                rule_type: r.rule_type,
                config: r.config,
                action_type: r.action_type,
                timeout_duration_seconds: r.timeout_duration_seconds,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleList {
                server_id: server_id.to_string(),
                rules,
            });
        }

        Ok(())
    }

    // ── Phase 7: Community & Discovery ─────────────────────────────

    // ── Invites ──

    /// Create a server invite. Requires CREATE_INVITES permission.
    pub async fn create_invite(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        max_uses: Option<i32>,
        expires_at: Option<&str>,
        channel_id: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let server_resource_id = referenced_server_id(server_id)?;
        let channel_resource_id = channel_id.map(referenced_channel_id).transpose()?;
        let created = self
            .community_service()?
            .create_invite(
                &actor,
                &server_resource_id,
                max_uses,
                expires_at,
                channel_resource_id.as_ref(),
            )
            .await
            .map_err(String::from)?;

        let invite = InviteInfo {
            id: created.id,
            code: created.code,
            server_id: server_id.to_string(),
            created_by: actor.user_id().as_str().to_owned(),
            max_uses,
            use_count: 0,
            expires_at: expires_at.map(String::from),
            channel_id: channel_id.map(String::from),
            created_at: created.created_at,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteCreate {
                server_id: server_id.to_string(),
                invite,
            });
        }

        Ok(())
    }

    /// List invites for a server. Requires MANAGE_SERVER permission.
    pub async fn list_invites(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (rows, stamp) = self
            .community_service()?
            .list_invites(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let invites: Vec<InviteInfo> = rows
            .into_iter()
            .map(|r| InviteInfo {
                id: r.id,
                code: r.code,
                server_id: r.server_id,
                created_by: r.created_by,
                max_uses: r.max_uses,
                use_count: r.use_count,
                expires_at: r.expires_at,
                channel_id: r.channel_id,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::InviteList {
                    server_id: server_id.to_string(),
                    invites,
                },
                Some(super::user_session::DeliveryGuard::Stamps(vec![stamp])),
            );
        }

        Ok(())
    }

    /// Delete an invite. Requires MANAGE_SERVER permission.
    pub async fn delete_invite(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        invite_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        self.community_service()?
            .delete_invite(&actor, &referenced_server_id(server_id)?, invite_id)
            .await
            .map_err(String::from)?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteDelete {
                server_id: server_id.to_string(),
                invite_id: invite_id.to_string(),
            });
        }

        Ok(())
    }

    /// Use an invite code to join a server. Any authenticated user can use this.
    pub async fn use_invite(&self, session_id: ConnectionId, code: &str) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let redeemed = self
            .community_service()?
            .redeem_invite(&actor, code)
            .await?;
        let server_id = redeemed.server_id.into_inner();
        if let Some(mut server) = self.servers.get_mut(&server_id) {
            server.member_user_ids.insert(user_id.clone());
        }

        // Auto-join default channel (#general)
        let default_channel = self
            .channel_name_index
            .get(&(server_id.clone(), "#general".to_string()))
            .map(|r| r.clone());
        if default_channel.is_some() {
            let _ = self.join_channel(session_id, &server_id, "#general").await;
        }

        // Send updated server list to the user
        let servers = self.list_servers_for_user(&user_id).await;
        let _ = session.send(ChatEvent::ServerList { servers });

        Ok(())
    }

    // ── Events ──

    /// Create a scheduled server event. Requires MANAGE_SERVER permission.
    pub async fn create_event(
        &self,
        session_id: ConnectionId,
        params: &CreateServerEventRequest<'_>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let server_resource_id = referenced_server_id(params.server_id)?;
        let channel_resource_id = params.channel_id.map(referenced_channel_id).transpose()?;
        let created_at = self
            .community_service()?
            .create_event(
                &actor,
                &super::community_service::CreateEvent {
                    id: params.id,
                    server_id: &server_resource_id,
                    name: params.name,
                    description: params.description,
                    channel_id: channel_resource_id.as_ref(),
                    start_time: params.start_time,
                    end_time: params.end_time,
                    image_url: params.image_url,
                    created_by: params.created_by,
                },
            )
            .await
            .map_err(String::from)?;

        let event_info = EventInfo {
            id: params.id.to_string(),
            server_id: params.server_id.to_string(),
            name: params.name.to_string(),
            description: params.description.map(String::from),
            channel_id: params.channel_id.map(String::from),
            start_time: params.start_time.to_string(),
            end_time: params.end_time.map(String::from),
            image_url: params.image_url.map(String::from),
            created_by: params.created_by.to_string(),
            status: "scheduled".to_string(),
            interested_count: 0,
            created_at,
        };

        let event = ChatEvent::EventUpdate {
            server_id: params.server_id.to_string(),
            event: event_info,
        };
        self.broadcast_to_server(params.server_id, &event);

        Ok(())
    }

    /// List events for a server. Requires VIEW_CHANNELS permission.
    pub async fn list_events(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (rows, stamp) = self
            .community_service()?
            .list_events(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let mut events = Vec::with_capacity(rows.len());
        for visible in rows {
            let row = visible.event;
            events.push(EventInfo {
                id: row.id,
                server_id: row.server_id,
                name: row.name,
                description: row.description,
                channel_id: row.channel_id,
                start_time: row.start_time,
                end_time: row.end_time,
                image_url: row.image_url,
                created_by: row.created_by,
                status: row.status,
                interested_count: visible.rsvp_count,
                created_at: row.created_at,
            });
        }

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::EventList {
                    server_id: server_id.to_string(),
                    events,
                },
                Some(super::user_session::DeliveryGuard::Stamps(vec![stamp])),
            );
        }

        Ok(())
    }

    /// Update an event's status. Requires MANAGE_SERVER permission.
    pub async fn update_event_status(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), String> {
        if !["scheduled", "active", "completed", "cancelled"].contains(&status) {
            return Err("Invalid status. Must be: scheduled, active, completed, cancelled".into());
        }
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let row = self
            .community_service()?
            .update_event_status(&actor, &referenced_server_id(server_id)?, event_id, status)
            .await?;
        let rsvp_count = crate::db::queries::events::get_rsvp_count(
            self.db.as_ref().ok_or("No database configured")?,
            event_id,
        )
        .await
        .unwrap_or(0);
        let event_info = EventInfo {
            id: row.id,
            server_id: row.server_id,
            name: row.name,
            description: row.description,
            channel_id: row.channel_id,
            start_time: row.start_time,
            end_time: row.end_time,
            image_url: row.image_url,
            created_by: row.created_by,
            status: row.status,
            interested_count: rsvp_count,
            created_at: row.created_at,
        };
        self.broadcast_to_server(
            server_id,
            &ChatEvent::EventUpdate {
                server_id: server_id.to_string(),
                event: event_info,
            },
        );
        Ok(())
    }

    /// Delete a scheduled event. Requires MANAGE_SERVER permission.
    pub async fn delete_event(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        self.community_service()?
            .delete_event(&actor, &referenced_server_id(server_id)?, event_id)
            .await?;
        self.broadcast_to_server(
            server_id,
            &ChatEvent::EventDelete {
                server_id: server_id.to_string(),
                event_id: event_id.to_string(),
            },
        );
        Ok(())
    }

    /// Set an RSVP for an event. Requires visibility of the linked channel.
    pub async fn set_rsvp(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), String> {
        if !["interested", "going", "not_going"].contains(&status) {
            return Err("Invalid RSVP status. Must be: interested, going, not_going".into());
        }
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (channel_id, rows) = self
            .community_service()?
            .set_rsvp(
                &actor,
                &referenced_server_id(server_id)?,
                event_id,
                (status != "not_going").then_some(status),
            )
            .await?;
        let rsvps = rows
            .into_iter()
            .map(|row| RsvpInfo {
                user_id: row.user_id,
                status: row.status,
            })
            .collect();
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::EventRsvpList {
                    event_id: event_id.to_string(),
                    rsvps,
                },
                Some(match channel_id {
                    Some(channel_id) => {
                        super::user_session::DeliveryGuard::Channels(vec![channel_id.into_inner()])
                    }
                    None => super::user_session::DeliveryGuard::ServerMembership(vec![
                        server_id.to_string(),
                    ]),
                }),
            );
        }
        Ok(())
    }

    /// Remove an RSVP for an event. Requires visibility of the linked channel.
    pub async fn remove_rsvp(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        self.community_service()?
            .set_rsvp(&actor, &referenced_server_id(server_id)?, event_id, None)
            .await?;
        Ok(())
    }

    /// List RSVPs for an event. Sends EventRsvpList to the requesting session.
    pub async fn list_rsvps(&self, session_id: ConnectionId, event_id: &str) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let (server_id, channel_id, rows) = self
            .community_service()?
            .list_rsvps(&actor, event_id)
            .await?;
        let rsvps = rows
            .into_iter()
            .map(|row| RsvpInfo {
                user_id: row.user_id,
                status: row.status,
            })
            .collect();
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::EventRsvpList {
                    event_id: event_id.to_string(),
                    rsvps,
                },
                Some(match channel_id {
                    Some(channel_id) => {
                        super::user_session::DeliveryGuard::Channels(vec![channel_id.into_inner()])
                    }
                    None => super::user_session::DeliveryGuard::ServerMembership(vec![
                        server_id.into_inner(),
                    ]),
                }),
            );
        }
        Ok(())
    }

    // ── Community ──

    /// Update community/discovery settings. Requires MANAGE_SERVER permission.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_community_settings(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        description: Option<&str>,
        is_discoverable: bool,
        welcome_message: Option<&str>,
        rules_text: Option<&str>,
        category: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let rules_accepted = self
            .community_service()?
            .update_community(
                &actor,
                &super::community_service::UpdateCommunityParams {
                    server_id: &referenced_server_id(server_id)?,
                    description,
                    discoverable: is_discoverable,
                    welcome: welcome_message,
                    rules: rules_text,
                    category,
                },
            )
            .await
            .map_err(String::from)?;

        let community = ServerCommunityInfo {
            server_id: server_id.to_string(),
            description: description.map(String::from),
            is_discoverable,
            welcome_message: welcome_message.map(String::from),
            rules_text: rules_text.map(String::from),
            category: category.map(String::from),
            rules_accepted: Some(rules_accepted),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerCommunity { community });
        }

        Ok(())
    }

    /// Get community/discovery settings for a server. Requires VIEW_CHANNELS permission.
    pub async fn get_community_settings(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (server, rules_accepted, stamp) = self
            .community_service()?
            .get_community(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let community = ServerCommunityInfo {
            server_id: server.id,
            description: server.description,
            is_discoverable: server.is_discoverable != 0,
            welcome_message: server.welcome_message,
            rules_text: server.rules_text,
            category: server.category,
            rules_accepted: Some(rules_accepted),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::ServerCommunity { community },
                Some(super::user_session::DeliveryGuard::Stamps(vec![stamp])),
            );
        }

        Ok(())
    }

    /// Discover public servers, optionally filtered by category. No permission needed.
    pub async fn discover_servers(
        &self,
        session_id: ConnectionId,
        category: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let rows = self
            .community_service()?
            .discover(&actor, category)
            .await
            .map_err(String::from)?;

        let servers: Vec<ServerCommunityInfo> = rows
            .into_iter()
            .map(|r| ServerCommunityInfo {
                server_id: r.id,
                description: r.description,
                is_discoverable: r.is_discoverable != 0,
                welcome_message: r.welcome_message,
                rules_text: r.rules_text,
                category: r.category,
                rules_accepted: None,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::DiscoverServers { servers });
        }

        Ok(())
    }

    /// Accept server rules as a member.
    pub async fn accept_rules(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.community_service()?
            .accept_rules(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        Ok(())
    }

    pub async fn set_vanity_code(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        vanity_code: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.community_service()?
            .set_vanity_code(&actor, &referenced_server_id(server_id)?, vanity_code)
            .await
            .map_err(String::from)
    }

    // ── Announcements ──

    /// Set a channel as an announcement channel. Requires MANAGE_CHANNELS permission.
    pub async fn set_announcement_channel(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        is_announcement: bool,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        self.community_service()?
            .set_announcement(
                &actor,
                &referenced_server_id(server_id)?,
                &referenced_channel_id(&channel_id)?,
                is_announcement,
            )
            .await
            .map_err(String::from)?;

        Ok(())
    }

    /// Follow an announcement channel, cross-posting to a target channel.
    /// Requires MANAGE_CHANNELS permission on the target server.
    pub async fn follow_channel(
        &self,
        session_id: ConnectionId,
        source_channel_id: &str,
        target_channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let created = self
            .community_service()?
            .follow_channel(
                &actor,
                &referenced_channel_id(source_channel_id)?,
                &referenced_channel_id(target_channel_id)?,
            )
            .await
            .map_err(String::from)?;

        let follow = ChannelFollowInfo {
            id: created.id,
            source_channel_id: source_channel_id.to_string(),
            target_channel_id: target_channel_id.to_string(),
            created_by: created.created_by,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ChannelFollowCreate { follow });
        }

        Ok(())
    }

    /// Unfollow an announcement channel. Requires MANAGE_CHANNELS permission.
    pub async fn unfollow_channel(
        &self,
        session_id: ConnectionId,
        follow_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let target_server = self
            .community_service()?
            .unfollow_channel(&actor, follow_id)
            .await
            .map_err(String::from)?;

        let session = self.get_session(session_id).ok_or("Session not found")?;
        let _ = session.send_guarded(
            ChatEvent::ChannelFollowDelete {
                follow_id: follow_id.to_string(),
            },
            Some(super::user_session::DeliveryGuard::ServerPermissions(vec![
                (target_server.into_inner(), Permissions::MANAGE_CHANNELS),
            ])),
        );

        Ok(())
    }

    /// List follows for an announcement channel. Sends ChannelFollowList to the session.
    pub async fn list_channel_follows(
        &self,
        session_id: ConnectionId,
        channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (rows, stamp) = self
            .community_service()?
            .list_channel_follows(&actor, &referenced_channel_id(channel_id)?)
            .await
            .map_err(String::from)?;

        let follows: Vec<ChannelFollowInfo> = rows
            .into_iter()
            .map(|r| ChannelFollowInfo {
                id: r.id,
                source_channel_id: r.source_channel_id,
                target_channel_id: r.target_channel_id,
                created_by: r.created_by,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::ChannelFollowList {
                    channel_id: channel_id.to_string(),
                    follows,
                },
                Some(super::user_session::DeliveryGuard::Stamps(vec![stamp])),
            );
        }

        Ok(())
    }

    /// Explicitly publish a message from a public announcement channel.
    pub async fn publish_announcement(
        &self,
        session_id: ConnectionId,
        message_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let publications = self
            .messaging_service()
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?
            .publish_announcement(
                &actor,
                super::messaging::PublishAnnouncementCommand { message_id },
            )
            .await
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AnnouncementPublished {
                source_message_id: message_id.to_string(),
                published_count: publications.len(),
            });
        }
        Ok(())
    }

    // ── Templates ──

    /// Create a server template (snapshot of channels, categories, roles).
    /// Requires MANAGE_SERVER permission.
    pub async fn create_template(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let created = self
            .community_service()?
            .create_template(&actor, &referenced_server_id(server_id)?, name, description)
            .await
            .map_err(String::from)?;
        let template = TemplateInfo {
            id: created.id,
            name: name.to_string(),
            description: description.map(String::from),
            server_id: server_id.to_string(),
            created_by: actor.user_id().as_str().to_owned(),
            use_count: 0,
            created_at: created.created_at,
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::TemplateUpdate {
                    server_id: server_id.to_string(),
                    template,
                },
                Some(super::user_session::DeliveryGuard::ServerPermissions(vec![
                    (server_id.to_string(), Permissions::MANAGE_SERVER),
                ])),
            );
        }
        Ok(())
    }

    /// List templates for a server. Sends TemplateList to the session.
    pub async fn list_templates(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (rows, stamp) = self
            .community_service()?
            .list_templates(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let templates: Vec<TemplateInfo> = rows
            .into_iter()
            .map(|r| TemplateInfo {
                id: r.id,
                name: r.name,
                description: r.description,
                server_id: r.server_id,
                created_by: r.created_by,
                use_count: r.use_count,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::TemplateList {
                    server_id: server_id.to_string(),
                    templates,
                },
                Some(super::user_session::DeliveryGuard::Stamps(vec![stamp])),
            );
        }

        Ok(())
    }

    /// Delete a template. Requires MANAGE_SERVER permission.
    pub async fn delete_template(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        template_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.community_service()?
            .delete_template(&actor, &referenced_server_id(server_id)?, template_id)
            .await
            .map_err(String::from)?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::TemplateDelete {
                    server_id: server_id.to_string(),
                    template_id: template_id.to_string(),
                },
                Some(super::user_session::DeliveryGuard::ServerPermissions(vec![
                    (server_id.to_string(), Permissions::MANAGE_SERVER),
                ])),
            );
        }

        Ok(())
    }

    /// Atomically create a server from a safe, versioned template snapshot.
    pub async fn instantiate_template(
        &self,
        session_id: ConnectionId,
        template_id: &str,
        server_name: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let server_id = self
            .community_service()?
            .instantiate_template(&actor, template_id, server_name)
            .await
            .map_err(String::from)?
            .into_inner();
        self.load_servers_from_db().await?;
        self.load_channels_from_db().await?;
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::TemplateInstantiated {
                template_id: template_id.to_string(),
                server_id,
            });
        }
        Ok(())
    }

    // ── Phase 8: Integrations & Bots ──

    /// Create a webhook for a channel. Requires MANAGE_SERVER permission.
    pub async fn create_webhook(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_id: &str,
        name: &str,
        webhook_type: &str,
        url: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let created = self
            .integration_service()?
            .create_webhook(
                &actor,
                super::integrations::CreateWebhook {
                    server_id,
                    channel_id,
                    name,
                    webhook_type,
                    url,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        let mut webhook = webhook_row_to_info(created.row);
        self.broadcast_to_server(
            server_id,
            &ChatEvent::WebhookUpdate {
                server_id: server_id.to_owned(),
                webhook: webhook.clone(),
            },
        );
        webhook.token = created.one_time_secret;
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::WebhookUpdate {
                server_id: server_id.to_owned(),
                webhook,
            });
        }
        Ok(())
    }

    /// List webhooks for a server. Requires MANAGE_SERVER permission.
    pub async fn list_webhooks(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::webhooks::list_webhooks(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list webhooks: {e}"))?;

        let webhooks: Vec<WebhookInfo> = rows.into_iter().map(webhook_row_to_info).collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::WebhookList {
                server_id: server_id.to_string(),
                webhooks,
            });
        }

        Ok(())
    }

    /// Update a webhook.
    pub async fn update_webhook(
        &self,
        session_id: ConnectionId,
        webhook_id: &str,
        name: &str,
        avatar_url: Option<&str>,
        channel_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let updated = self
            .integration_service()?
            .update_webhook(&actor, webhook_id, name, avatar_url, channel_id)
            .await
            .map_err(|error| error.to_string())?;
        let server_id = updated.server_id.clone();
        self.broadcast_to_server(
            &server_id,
            &ChatEvent::WebhookUpdate {
                server_id: server_id.clone(),
                webhook: webhook_row_to_info(updated),
            },
        );
        Ok(())
    }

    /// Delete a webhook.
    pub async fn delete_webhook(
        &self,
        session_id: ConnectionId,
        webhook_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let server_id = self
            .integration_service()?
            .delete_webhook(&actor, webhook_id)
            .await
            .map_err(|error| error.to_string())?;
        self.broadcast_to_server(
            &server_id,
            &ChatEvent::WebhookDelete {
                server_id: server_id.clone(),
                webhook_id: webhook_id.to_owned(),
            },
        );
        Ok(())
    }

    pub async fn list_webhook_deliveries(
        &self,
        actor: &crate::auth::authority::Actor,
        webhook_id: &str,
        limit: i64,
    ) -> Result<Vec<super::integrations::WebhookDeliveryStatus>, String> {
        self.integration_service()?
            .list_deliveries(actor, webhook_id, limit)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn enqueue_webhook_test(
        &self,
        actor: &crate::auth::authority::Actor,
        webhook_id: &str,
    ) -> Result<String, String> {
        self.integration_service()?
            .enqueue_test_delivery(actor, webhook_id)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn retry_webhook_delivery(
        &self,
        actor: &crate::auth::authority::Actor,
        delivery_id: &str,
    ) -> Result<(), String> {
        self.integration_service()?
            .retry_delivery(actor, delivery_id)
            .await
            .map_err(|error| error.to_string())
    }

    /// Create a bot account. Only authenticated users can create bots.
    pub async fn create_bot(
        &self,
        session_id: ConnectionId,
        username: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), String> {
        let creator_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        validation::validate_nickname(username)?;

        let bot_user_id = Uuid::new_v4().to_string();
        crate::db::queries::bots::create_bot_user_owned(
            pool,
            &bot_user_id,
            username,
            avatar_url,
            &creator_id,
        )
        .await
        .map_err(|e| format!("Failed to create bot: {e}"))?;

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let bot_id = crate::auth::authority::UserId::from_stored(bot_user_id.clone())
            .map_err(|e| e.to_string())?;
        let issued = match auth
            .issue_bot_token(&bot_id, "Default", "bot messages")
            .await
        {
            Ok(issued) => issued,
            Err(error) => {
                let _ = crate::db::queries::bots::delete_bot_user(pool, &bot_user_id).await;
                return Err(format!("Failed to create bot token: {error}"));
            }
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotCredentialCreated {
                bot_user_id: bot_user_id.clone(),
                token: issued.secret,
                credential: BotTokenInfo {
                    id: issued.token_id,
                    name: "Default".into(),
                    scopes: "bot messages".into(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    last_used: None,
                },
            });
        }
        self.list_owned_bots(session_id).await
    }

    pub async fn list_owned_bots(&self, session_id: ConnectionId) -> Result<(), String> {
        use sqlx::Row;
        let owner_id = self.get_user_id(session_id)?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = sqlx::query(
            "SELECT u.id,u.username,u.avatar_url,i.server_id
             FROM bot_ownership o JOIN users u ON u.id=o.bot_user_id
             LEFT JOIN bot_installations i ON i.bot_user_id=u.id AND i.state='active'
             WHERE o.owner_user_id=? AND o.repair_required=0
             ORDER BY u.username,u.id,i.server_id",
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("Failed to list bots: {error}"))?;
        let mut bots: Vec<crate::engine::events::BotAccountInfo> = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            if bots.last().is_none_or(|bot| bot.id != id) {
                bots.push(crate::engine::events::BotAccountInfo {
                    id,
                    username: row.get(1),
                    avatar_url: row.get(2),
                    installed_server_ids: Vec::new(),
                });
            }
            if let Some(server_id) = row.get::<Option<String>, _>(3) {
                bots.last_mut()
                    .expect("bot was inserted")
                    .installed_server_ids
                    .push(server_id);
            }
        }
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotAccountList { bots });
        }
        Ok(())
    }

    /// Create a new token for a bot. Caller must own the bot.
    pub async fn create_bot_token(
        &self,
        session_id: ConnectionId,
        bot_user_id: &str,
        name: &str,
        scopes: Option<&str>,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let caller_id = self.get_user_id(session_id)?;
        let owner_id = crate::db::queries::bots::bot_owner(pool, bot_user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if owner_id.as_deref() != Some(&caller_id) {
            return Err("FORBIDDEN: only the recorded bot owner may create credentials".into());
        }

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let bot_id =
            crate::auth::authority::UserId::from_stored(bot_user_id).map_err(|e| e.to_string())?;
        let issued = auth
            .issue_bot_token(&bot_id, name, scopes.unwrap_or("bot messages"))
            .await
            .map_err(|e| format!("Failed to create bot token: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotCredentialCreated {
                bot_user_id: bot_user_id.to_owned(),
                token: issued.secret,
                credential: BotTokenInfo {
                    id: issued.token_id,
                    name: name.to_owned(),
                    scopes: scopes.unwrap_or("bot messages").to_owned(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    last_used: None,
                },
            });
        }

        Ok(())
    }

    /// List bot tokens (without hashes).
    pub async fn list_bot_tokens(
        &self,
        session_id: ConnectionId,
        bot_user_id: &str,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let caller_id = self.get_user_id(session_id)?;
        let owner_id = crate::db::queries::bots::bot_owner(pool, bot_user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if owner_id.as_deref() != Some(&caller_id) {
            return Err("FORBIDDEN: only the recorded bot owner may list credentials".into());
        }
        let rows = crate::db::queries::bots::list_bot_tokens(pool, bot_user_id)
            .await
            .map_err(|e| format!("Failed to list bot tokens: {e}"))?;

        let tokens: Vec<BotTokenInfo> = rows
            .into_iter()
            .map(|r| BotTokenInfo {
                id: r.id,
                name: r.name,
                scopes: r.scopes,
                created_at: r.created_at,
                last_used: r.last_used,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotTokenList {
                bot_user_id: bot_user_id.to_string(),
                tokens,
            });
        }

        Ok(())
    }

    /// Delete a bot token.
    pub async fn delete_bot_token(
        &self,
        session_id: ConnectionId,
        token_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let user_id = actor.user_id().as_str().to_owned();

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let owner_id = crate::db::queries::bots::bot_token_owner(pool, token_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if owner_id.as_deref() != Some(&user_id) {
            return Err("FORBIDDEN: only the recorded bot owner may revoke credentials".into());
        }
        let bot_user_id: String = sqlx::query_scalar("SELECT user_id FROM bot_tokens WHERE id=?")
            .bind(token_id)
            .fetch_one(pool)
            .await
            .map_err(|error| error.to_string())?;

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        auth.revoke_bot_token(token_id)
            .await
            .map_err(|e| format!("Failed to revoke bot token: {e}"))?;

        self.list_bot_tokens(session_id, &bot_user_id).await?;
        Ok(())
    }

    /// Add a bot to a server. Requires MANAGE_SERVER permission.
    pub async fn add_bot_to_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        bot_user_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::add_bot_to_server_with_grants(
            pool,
            server_id,
            bot_user_id,
            &self.get_user_id(session_id)?,
            "commands messages",
        )
        .await
        .map_err(|e| format!("Failed to add bot to server: {e}"))?;

        self.list_owned_bots(session_id).await
    }

    /// Remove a bot from a server. Requires MANAGE_SERVER permission.
    pub async fn remove_bot_from_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        bot_user_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::remove_bot_from_server(pool, server_id, bot_user_id)
            .await
            .map_err(|e| format!("Failed to remove bot from server: {e}"))?;

        self.list_owned_bots(session_id).await
    }

    /// Register a slash command for a bot. Caller must own the bot.
    pub async fn register_slash_command(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        description: &str,
        options_json: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_bot_installation_scope(auth, &actor, server_id, "commands")
            .await
            .map_err(|error| error.to_string())?;

        if name.is_empty()
            || name.len() > 32
            || !name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
        {
            return Err("Command name must be 1-32 lowercase letters, digits, '_' or '-'".into());
        }
        if description.is_empty()
            || description.len() > 100
            || description.chars().any(char::is_control)
        {
            return Err("Command description must be 1-100 printable characters".into());
        }

        let id = Uuid::new_v4().to_string();
        let opts = options_json.unwrap_or("[]");
        // Validate JSON
        let options = serde_json::from_str::<Vec<SlashCommandOption>>(opts)
            .map_err(|e| format!("Invalid options JSON: {e}"))?;
        validate_slash_command_options(&options)?;

        use crate::db::models::CreateSlashCommandParams;
        let params = CreateSlashCommandParams {
            id: &id,
            bot_user_id: actor.user_id().as_str(),
            server_id: Some(server_id),
            name,
            description,
            options_json: opts,
        };

        crate::db::queries::slash_commands::create_command(pool, &params)
            .await
            .map_err(|e| format!("Failed to register command: {e}"))?;

        let cmd = SlashCommandInfo {
            id: id.clone(),
            bot_user_id: actor.user_id().as_str().to_owned(),
            name: name.to_string(),
            description: description.to_string(),
            options,
        };

        self.broadcast_to_server(
            server_id,
            &ChatEvent::SlashCommandUpdate {
                server_id: server_id.to_string(),
                command: cmd,
            },
        );

        Ok(())
    }

    /// List slash commands available in a server.
    pub async fn list_slash_commands(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::VIEW_CHANNELS)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::slash_commands::list_commands_for_server(pool, server_id)
            .await
            .map_err(|e| format!("Failed to list commands: {e}"))?;

        let commands: Vec<SlashCommandInfo> = rows
            .into_iter()
            .map(|r| {
                let options: Vec<SlashCommandOption> =
                    serde_json::from_str(&r.options_json).unwrap_or_default();
                SlashCommandInfo {
                    id: r.id,
                    bot_user_id: r.bot_user_id,
                    name: r.name,
                    description: r.description,
                    options,
                }
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::SlashCommandList {
                server_id: server_id.to_string(),
                commands,
            });
        }

        Ok(())
    }

    /// Delete a slash command.
    pub async fn delete_slash_command(
        &self,
        session_id: ConnectionId,
        command_id: &str,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let cmd = crate::db::queries::slash_commands::get_command(pool, command_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Command not found")?;

        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        // An installed command-scoped bot may remove its own registration;
        // server managers may remove any command from their server.
        if let Some(sid) = &cmd.server_id {
            if actor.user_id().as_str() == cmd.bot_user_id {
                let auth = self
                    .auth
                    .get()
                    .ok_or("Credential authority is not configured")?;
                super::authorization::AuthorizationService::new(pool.clone())
                    .authorize_bot_installation_scope(auth, &actor, sid, "commands")
                    .await
                    .map_err(|error| error.to_string())?;
            } else {
                self.require_permission(session_id, sid, None, Permissions::MANAGE_SERVER)
                    .await?;
            }
        } else {
            // Global command with no server — just verify authentication.
            let _user_id = self.get_user_id(session_id)?;
        }

        crate::db::queries::slash_commands::delete_command(pool, command_id)
            .await
            .map_err(|e| format!("Failed to delete command: {e}"))?;

        if let Some(sid) = &cmd.server_id {
            self.broadcast_to_server(
                sid,
                &ChatEvent::SlashCommandDelete {
                    server_id: sid.clone(),
                    command_id: command_id.to_string(),
                },
            );
        }

        Ok(())
    }

    /// Invoke a slash command. Creates an interaction and dispatches to the bot.
    pub async fn invoke_slash_command(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel: &str,
        command_name: &str,
        args_json: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let user_id = actor.user_id().as_str().to_owned();

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        // Resolve channel_id from name (normalize for case-insensitive lookup)
        let channel = normalize_channel_name(channel);
        let channel_id = self.resolve_channel_id(server_id, &channel)?;
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let (_permit, mut transaction) = self.begin_admitted_write().await?;
        let cmd = sqlx::query_as::<_, crate::db::models::SlashCommandRow>(
            "SELECT c.* FROM slash_commands c \
             JOIN bot_installations i ON i.bot_user_id=c.bot_user_id AND i.server_id=? \
             WHERE (c.server_id=? OR c.server_id IS NULL) AND c.name=? COLLATE NOCASE \
               AND i.state='active' AND i.revoked_at IS NULL \
               AND (instr(' '||i.granted_scopes||' ',' commands ')>0 \
                    OR instr(' '||i.granted_scopes||' ',' * ')>0)",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(command_name)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?
        .ok_or_else(|| format!("NOT_FOUND: unknown command /{command_name}"))?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                &actor,
                &channel_id,
                super::authorization::ChannelAction::Send,
            )
            .await
            .map_err(|error| error.to_string())?;

        let interaction_id = Uuid::new_v4().to_string();
        let data: serde_json::Value = match args_json {
            Some(value) if value.len() <= 8 * 1024 => serde_json::from_str(value)
                .map_err(|_| "Command arguments must be valid JSON".to_string())?,
            Some(_) => return Err("Command arguments are too large".into()),
            None => serde_json::Value::Object(serde_json::Map::new()),
        };
        if !data.is_object() {
            return Err("Command arguments must be a JSON object".into());
        }
        let options: Vec<SlashCommandOption> = serde_json::from_str(&cmd.options_json)
            .map_err(|_| "Command definition is unavailable".to_string())?;
        validate_slash_command_arguments(&options, &data)?;
        let arguments = data.as_object().expect("argument object was validated");
        for option in &options {
            let Some(value) = arguments
                .get(&option.name)
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let exists = match option.option_type.as_str() {
                "user" => sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
                )
                .bind(server_id)
                .bind(value)
                .fetch_one(&mut *transaction)
                .await,
                "channel" => {
                    sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(SELECT 1 FROM channels WHERE server_id=? AND id=?)",
                    )
                    .bind(server_id)
                    .bind(value)
                    .fetch_one(&mut *transaction)
                    .await
                }
                "role" => {
                    sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(SELECT 1 FROM roles WHERE server_id=? AND id=?)",
                    )
                    .bind(server_id)
                    .bind(value)
                    .fetch_one(&mut *transaction)
                    .await
                }
                _ => continue,
            }
            .map_err(|error| format!("DB error: {error}"))?;
            if !exists {
                return Err(format!("Unknown value for command option: {}", option.name));
            }
        }

        let data_str = serde_json::to_string(&data).unwrap_or_default();
        let interaction_expires_at = (Utc::now() + chrono::Duration::minutes(15))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let interaction_params = crate::db::models::CreateInteractionParams {
            id: &interaction_id,
            interaction_type: "slash_command",
            command_id: Some(&cmd.id),
            user_id: &user_id,
            server_id,
            channel_id: &channel_id,
            data_json: &data_str,
            application_user_id: &cmd.bot_user_id,
            expires_at: &interaction_expires_at,
        };
        sqlx::query(
            "INSERT INTO interactions \
             (id,interaction_type,command_id,user_id,server_id,channel_id,data_json, \
              application_user_id,expires_at,response_state) \
             VALUES(?, 'slash_command', ?, ?, ?, ?, ?, ?, ?, 'pending')",
        )
        .bind(interaction_params.id)
        .bind(interaction_params.command_id)
        .bind(interaction_params.user_id)
        .bind(interaction_params.server_id)
        .bind(interaction_params.channel_id)
        .bind(interaction_params.data_json)
        .bind(interaction_params.application_user_id)
        .bind(interaction_params.expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;
        transaction
            .commit()
            .await
            .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;

        let interaction = InteractionInfo {
            id: interaction_id.clone(),
            interaction_type: "slash_command".to_string(),
            command_name: Some(command_name.to_string()),
            user_id: user_id.clone(),
            server_id: server_id.to_string(),
            channel_id: channel_id.clone(),
            data,
        };

        // Send to the bot (find sessions for the bot user)
        for entry in self.sessions.iter() {
            let s = entry.value();
            if let Some(ref uid) = s.user_id
                && uid == &cmd.bot_user_id
            {
                let _ = s.send_guarded(
                    ChatEvent::InteractionCreate {
                        interaction: interaction.clone(),
                    },
                    Some(super::user_session::DeliveryGuard::BotInstallationScopes(
                        vec![(server_id.to_owned(), "commands".to_owned())],
                    )),
                );
            }
        }

        // Also send a notice to the invoker
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerNotice {
                message: format!("/{command_name} invoked"),
            });
        }

        Ok(())
    }

    /// Invoke a button or select menu from a persisted bot response.
    pub async fn invoke_message_component(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        custom_id: &str,
        values: &[String],
    ) -> Result<(), String> {
        if message_id.is_empty()
            || message_id.len() > 128
            || custom_id.is_empty()
            || custom_id.len() > 100
            || values.len() > 25
            || values.iter().any(|value| value.len() > 100)
        {
            return Err("Invalid message component invocation".into());
        }
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let (_permit, mut transaction) = self.begin_admitted_write().await?;
        use sqlx::Row;
        let (server_id, channel_id, application_user_id, components): (
            String,
            String,
            String,
            Vec<super::events::MessageComponent>,
        ) = if let Some(interaction_id) = message_id.strip_prefix("ephemeral:") {
            let row = sqlx::query(
                "SELECT server_id,channel_id,application_user_id,ephemeral_response_json \
                     FROM interactions WHERE id=? AND user_id=? AND response_state='responded' \
                       AND response_expires_at IS NOT NULL \
                       AND response_expires_at>datetime('now')",
            )
            .bind(interaction_id)
            .bind(actor.user_id().as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| format!("DB error: {error}"))?
            .ok_or("Message component not found")?;
            let response: InteractionResponseData = serde_json::from_str(row.get::<&str, _>(3))
                .map_err(|_| "Message component is unavailable".to_string())?;
            (
                row.get(0),
                row.get(1),
                row.get(2),
                response.components.unwrap_or_default(),
            )
        } else {
            let row = sqlx::query(
                "SELECT m.server_id,m.channel_id,m.components_json,i.application_user_id \
                     FROM messages m JOIN interactions i ON i.response_message_id=m.id \
                     WHERE m.id=? AND m.deleted_at IS NULL",
            )
            .bind(message_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| format!("DB error: {error}"))?
            .ok_or("Message component not found")?;
            let components = serde_json::from_str(row.get::<&str, _>(2))
                .map_err(|_| "Message component is unavailable".to_string())?;
            (row.get(0), row.get(1), row.get(3), components)
        };
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                &actor,
                &channel_id,
                ChannelAction::View,
            )
            .await
            .map_err(|error| error.to_string())?;
        let component =
            find_message_component(&components, custom_id).ok_or("Message component not found")?;
        let interaction_type = match component {
            super::events::MessageComponent::Button { disabled, .. } => {
                if *disabled || !values.is_empty() {
                    return Err("Message component is unavailable".into());
                }
                "button"
            }
            super::events::MessageComponent::SelectMenu {
                options,
                min_values,
                max_values,
                ..
            } => {
                let distinct: std::collections::HashSet<_> = values.iter().collect();
                if *min_values < 0
                    || *max_values < *min_values
                    || values.len() < *min_values as usize
                    || values.len() > *max_values as usize
                    || distinct.len() != values.len()
                {
                    return Err("Invalid select menu values".into());
                }
                if values
                    .iter()
                    .any(|value| !options.iter().any(|option| option.value == *value))
                {
                    return Err("Invalid select menu values".into());
                }
                "select_menu"
            }
            super::events::MessageComponent::ActionRow { .. } => {
                return Err("Message component not found".into());
            }
        };
        let installation_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM bot_installations \
             WHERE bot_user_id=? AND server_id=? AND state='active' AND revoked_at IS NULL \
             AND (instr(' '||granted_scopes||' ',' commands ')>0 \
                  OR instr(' '||granted_scopes||' ',' * ')>0))",
        )
        .bind(&application_user_id)
        .bind(&server_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| format!("DB error: {error}"))?;
        if !installation_active {
            return Err("Message component is unavailable".into());
        }
        let interaction_id = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() + chrono::Duration::minutes(15))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let data = serde_json::json!({
            "message_id": message_id,
            "custom_id": custom_id,
            "values": values,
        });
        let data_json = serde_json::to_string(&data)
            .map_err(|_| "Invalid message component invocation".to_string())?;
        sqlx::query(
            "INSERT INTO interactions \
             (id,interaction_type,user_id,server_id,channel_id,data_json, \
              application_user_id,expires_at,response_state) \
             VALUES(?,?,?,?,?,?,?,?, 'pending')",
        )
        .bind(&interaction_id)
        .bind(interaction_type)
        .bind(actor.user_id().as_str())
        .bind(&server_id)
        .bind(&channel_id)
        .bind(&data_json)
        .bind(&application_user_id)
        .bind(&expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;
        transaction
            .commit()
            .await
            .map_err(|error| format!("DEPENDENCY_UNAVAILABLE: {error}"))?;
        let interaction = InteractionInfo {
            id: interaction_id,
            interaction_type: interaction_type.to_owned(),
            command_name: None,
            user_id: actor.user_id().as_str().to_owned(),
            server_id: server_id.clone(),
            channel_id,
            data,
        };
        for entry in self.sessions.iter() {
            let session = entry.value();
            if session.user_id.as_deref() == Some(application_user_id.as_str()) {
                let _ = session.send_guarded(
                    ChatEvent::InteractionCreate {
                        interaction: interaction.clone(),
                    },
                    Some(super::user_session::DeliveryGuard::BotInstallationScopes(
                        vec![(server_id.clone(), "commands".to_owned())],
                    )),
                );
            }
        }
        Ok(())
    }

    /// Respond to an interaction (bot -> channel).
    pub async fn respond_to_interaction(
        &self,
        session_id: ConnectionId,
        interaction_id: &str,
        content: Option<&str>,
        embeds_json: Option<&str>,
        components_json: Option<&str>,
        ephemeral: bool,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let interaction = crate::db::queries::slash_commands::get_interaction(pool, interaction_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Interaction not found")?;

        if content.is_some_and(|value| value.len() > self.max_message_length)
            || embeds_json.is_some_and(|value| value.len() > 32 * 1024)
            || components_json.is_some_and(|value| value.len() > 32 * 1024)
        {
            return Err("Interaction response is too large".into());
        }
        let embeds = embeds_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| "Invalid interaction embeds".to_string())?;
        let components = components_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| "Invalid interaction components".to_string())?;
        if content.is_none() && embeds.is_none() && components.is_none() {
            return Err("Interaction response must contain content, embeds, or components".into());
        }
        if embeds
            .as_ref()
            .is_some_and(|values: &Vec<_>| values.len() > 10)
            || components
                .as_ref()
                .is_some_and(|values: &Vec<_>| values.len() > 5)
        {
            return Err("Interaction response contains too many embeds or components".into());
        }
        validate_rich_interaction_response(embeds.as_deref(), components.as_deref())?;

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        super::authorization::AuthorizationService::new(pool.clone())
            .authorize_bot_installation_scope(auth, &actor, &interaction.server_id, "commands")
            .await
            .map_err(|error| error.to_string())?;

        let response = InteractionResponseData {
            content: content.map(String::from),
            embeds: embeds.clone(),
            components: components.clone(),
            ephemeral,
        };

        // Resolve channel name from channel_id
        let channel_name = self
            .resolve_channel_name_from_id(&interaction.channel_id)
            .unwrap_or_else(|_| interaction.channel_id.clone());

        if ephemeral {
            let response_json = serde_json::to_string(&response)
                .map_err(|_| "Invalid interaction response".to_string())?;
            let response_expires_at = (Utc::now() + chrono::Duration::minutes(15))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let mut transaction = pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(|e| format!("Failed to begin interaction response: {e}"))?;
            use crate::db::queries::slash_commands::InteractionResponseResult;
            match crate::db::queries::slash_commands::accept_interaction_response(
                &mut transaction,
                interaction_id,
                actor.user_id().as_str(),
                None,
                Some(&response_json),
                Some(&response_expires_at),
            )
            .await
            .map_err(|e| format!("Failed to accept interaction response: {e}"))?
            {
                InteractionResponseResult::Accepted => transaction
                    .commit()
                    .await
                    .map_err(|e| format!("Failed to commit interaction response: {e}"))?,
                InteractionResponseResult::AlreadyResponded => {
                    return Err("Interaction already responded".into());
                }
                InteractionResponseResult::Expired => return Err("Interaction expired".into()),
                InteractionResponseResult::WrongApplication
                | InteractionResponseResult::NotFound => {
                    return Err("Interaction not found".into());
                }
            }
            // Send only to the invoker
            for entry in self.sessions.iter() {
                let s = entry.value();
                if let Some(ref uid) = s.user_id
                    && uid == &interaction.user_id
                {
                    let _ = s.send_guarded(
                        ChatEvent::InteractionResponse {
                            interaction_id: interaction_id.to_string(),
                            server_id: interaction.server_id.clone(),
                            channel: channel_name.clone(),
                            response: response.clone(),
                        },
                        Some(super::user_session::DeliveryGuard::Channels(vec![
                            interaction.channel_id.clone(),
                        ])),
                    );
                }
            }
        } else {
            let content = content.unwrap_or_default();
            let rich_embeds_json = embeds
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| "Invalid interaction embeds".to_string())?;
            let canonical_components_json = components
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| "Invalid interaction components".to_string())?;
            let request_id = Uuid::new_v4().to_string();
            let client_message_id = format!("interaction:{interaction_id}:response:1");
            let empty_attachments: Vec<String> = Vec::new();
            let receipt = self
                .messaging_service()
                .map_err(|error| error.to_string())?
                .respond_to_interaction_public(
                    &actor,
                    interaction_id,
                    super::messaging::SendMessageCommand {
                        request_id: &request_id,
                        client_message_id: &client_message_id,
                        operation_generation: None,
                        conversation_id: None,
                        server_id: &interaction.server_id,
                        channel: &channel_name,
                        content,
                        content_format: super::messaging::ContentFormat::Markdown,
                        reply_to_id: None,
                        attachment_ids: &empty_attachments,
                        mentions: &[],
                    },
                    rich_embeds_json.as_deref(),
                    canonical_components_json.as_deref(),
                )
                .await
                .map_err(|error| error.to_string())?;
            self.send_committed_receipt(session_id, &receipt);
        }

        Ok(())
    }

    /// Create an OAuth2 application.
    pub async fn create_oauth2_app(
        &self,
        session_id: ConnectionId,
        name: &str,
        description: Option<&str>,
        redirect_uris: &[String],
        client_type: &str,
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        if !matches!(client_type, "confidential" | "public")
            || name.trim().is_empty()
            || name.len() > 100
            || description.is_some_and(|value| value.len() > 1_000)
            || redirect_uris.is_empty()
            || redirect_uris.len() > 10
        {
            return Err("Invalid OAuth2 application registration".into());
        }
        let mut exact_redirects = Vec::with_capacity(redirect_uris.len());
        for redirect_uri in redirect_uris {
            if redirect_uri.len() > 2_048 || redirect_uri.contains('#') {
                return Err("Invalid OAuth2 redirect URI".into());
            }
            let parsed = reqwest::Url::parse(redirect_uri)
                .map_err(|_| "Invalid OAuth2 redirect URI".to_string())?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err("OAuth2 redirect URIs must use HTTPS without credentials".into());
            }
            if !exact_redirects.contains(redirect_uri) {
                exact_redirects.push(redirect_uri.clone());
            }
        }
        let id = Uuid::new_v4().to_string();
        let raw_secret =
            (client_type == "confidential").then(|| format!("secret_{}", Uuid::new_v4()));
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let secret_hash = match &raw_secret {
            Some(secret) => auth
                .hash_secret(secret.clone())
                .await
                .map_err(|_| "OAuth2 credential service unavailable".to_string())?,
            None => String::new(),
        };
        let uris_json = serde_json::to_string(&exact_redirects)
            .map_err(|e| format!("Invalid redirect URIs: {e}"))?;

        use crate::db::models::CreateOAuth2AppParams;
        let params = CreateOAuth2AppParams {
            id: &id,
            name,
            description: description.unwrap_or(""),
            icon_url: None,
            owner_id: &user_id,
            client_secret: &secret_hash,
            redirect_uris: &uris_json,
            scopes: "identify servers.read",
            client_type,
        };

        crate::db::queries::oauth2::create_app(pool, &params)
            .await
            .map_err(|e| format!("Failed to create OAuth2 app: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let app = OAuth2AppInfo {
                id: id.clone(),
                name: name.to_string(),
                description: description.unwrap_or("").to_string(),
                icon_url: None,
                owner_id: user_id,
                redirect_uris: exact_redirects,
                scopes: "identify servers.read".to_string(),
                is_public: client_type == "public",
                created_at: Utc::now().to_rfc3339(),
            };
            let _ = session.send(ChatEvent::OAuth2AppUpdate { app });
            let _ = session.send(ChatEvent::ServerNotice {
                message: raw_secret.map_or_else(
                    || format!("Public OAuth2 app created. Client ID: {id}"),
                    |secret| {
                        format!("OAuth2 app created! Client ID: {id}, Client Secret: {secret}")
                    },
                ),
            });
        }

        Ok(())
    }

    /// List OAuth2 apps owned by the current user.
    pub async fn list_oauth2_apps(&self, session_id: ConnectionId) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::oauth2::list_apps_by_owner(pool, &user_id)
            .await
            .map_err(|e| format!("Failed to list OAuth2 apps: {e}"))?;

        let apps: Vec<OAuth2AppInfo> = rows
            .into_iter()
            .map(|r| {
                let uris: Vec<String> = serde_json::from_str(&r.redirect_uris).unwrap_or_default();
                OAuth2AppInfo {
                    id: r.id,
                    name: r.name,
                    description: r.description,
                    icon_url: r.icon_url,
                    owner_id: r.owner_id,
                    redirect_uris: uris,
                    scopes: r.scopes,
                    is_public: r.is_public != 0,
                    created_at: r.created_at,
                }
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::OAuth2AppList { apps });
        }

        Ok(())
    }

    /// Delete an OAuth2 app. Only the owner can delete.
    pub async fn delete_oauth2_app(
        &self,
        session_id: ConnectionId,
        app_id: &str,
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let app = crate::db::queries::oauth2::get_app(pool, app_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("OAuth2 app not found")?;

        if app.owner_id != user_id {
            return Err("You can only delete your own apps".into());
        }

        crate::db::queries::oauth2::delete_app(pool, app_id)
            .await
            .map_err(|e| format!("Failed to delete app: {e}"))?;

        self.list_oauth2_apps(session_id).await
    }

    /// Execute an incoming webhook — post a message to the webhook's channel.
    /// No session required; the webhook token is the authentication.
    pub async fn execute_incoming_webhook(
        &self,
        webhook_id: &str,
        webhook_token: &str,
        content: &str,
        idempotency_key: &str,
        username_override: Option<&str>,
        avatar_override: Option<&str>,
    ) -> Result<super::messaging::CommandReceipt, String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let wh = crate::db::queries::webhooks::get_webhook(pool, webhook_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Invalid webhook")?;

        if wh.webhook_type != "incoming" {
            return Err("This endpoint is only for incoming webhooks".into());
        }
        if username_override.is_some() || avatar_override.is_some() {
            return Err("Webhook identity overrides are not supported".into());
        }
        if wh.credential_state != "active" || wh.revoked_at.is_some() {
            return Err("Invalid webhook token".into());
        }
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let actor = auth
            .authenticate_bot(webhook_token)
            .await
            .map_err(|_| "Invalid webhook token".to_string())?;
        if wh.credential_id.as_deref() != Some(actor.credential_id().as_str())
            || wh.principal_user_id.as_deref() != Some(actor.user_id().as_str())
        {
            return Err("Invalid webhook token".into());
        }
        let required_scope = format!("webhook:channel:{}", wh.channel_id);
        if !actor.scopes().contains(&required_scope) {
            return Err("Invalid webhook token".into());
        }
        let installation_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM bot_installations \
             WHERE bot_user_id=? AND server_id=? AND state='active' AND granted_scopes=?)",
        )
        .bind(actor.user_id().as_str())
        .bind(&wh.server_id)
        .bind(&required_scope)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to validate webhook grant: {e}"))?;
        if !installation_active {
            return Err("Invalid webhook token".into());
        }
        let conversation_id: String = sqlx::query_scalar(
            "SELECT id FROM conversations WHERE kind='channel' AND channel_id=?",
        )
        .bind(&wh.channel_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to resolve webhook channel: {e}"))?;
        let attachments = Vec::new();
        let mentions = Vec::new();
        self.messaging_service()
            .map_err(|error| error.to_string())?
            .send_channel_message(
                &actor,
                super::messaging::SendMessageCommand {
                    request_id: idempotency_key,
                    client_message_id: idempotency_key,
                    operation_generation: None,
                    conversation_id: Some(&conversation_id),
                    server_id: &wh.server_id,
                    channel: "",
                    content,
                    content_format: super::messaging::ContentFormat::Markdown,
                    reply_to_id: None,
                    attachment_ids: &attachments,
                    mentions: &mentions,
                },
            )
            .await
            .map_err(|error| error.to_string())
    }

    /// Helper: get user_id for a session.
    fn get_user_id(&self, session_id: ConnectionId) -> Result<String, String> {
        let session = self.sessions.get(&session_id).ok_or("Session not found")?;
        session
            .user_id
            .clone()
            .ok_or_else(|| "AUTH_REQUIRED".into())
    }

    /// Helper: resolve channel name from a channel_id by looking it up in self.channels.
    fn resolve_channel_name_from_id(&self, channel_id: &str) -> Result<String, String> {
        self.channels
            .get(channel_id)
            .map(|ch| ch.name.clone())
            .ok_or_else(|| format!("Channel ID {channel_id} not found"))
    }
}

fn group_member_roles(assignments: Vec<(String, Option<String>)>) -> Vec<MemberRoleInfo> {
    let mut by_user = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (user_id, role_id) in assignments {
        let role_ids = by_user.entry(user_id).or_default();
        if let Some(role_id) = role_id {
            role_ids.push(role_id);
        }
    }
    by_user
        .into_iter()
        .map(|(user_id, role_ids)| MemberRoleInfo { user_id, role_ids })
        .collect()
}

/// Convert a RoleRow to a RoleInfo for client consumption.
fn role_row_to_info(row: crate::db::models::RoleRow) -> RoleInfo {
    RoleInfo {
        id: row.id,
        server_id: row.server_id,
        name: row.name,
        color: row.color,
        icon_url: row.icon_url,
        position: row.position,
        permissions: row.permissions,
        is_default: row.is_default != 0,
    }
}

/// Convert a ChannelCategoryRow to a CategoryInfo for client consumption.
fn category_row_to_info(row: crate::db::models::ChannelCategoryRow) -> CategoryInfo {
    CategoryInfo {
        id: row.id,
        server_id: row.server_id,
        name: row.name,
        position: row.position,
    }
}

/// Convert a WebhookRow to a WebhookInfo for client consumption.
fn webhook_row_to_info(row: crate::db::models::WebhookRow) -> WebhookInfo {
    WebhookInfo {
        id: row.id,
        server_id: row.server_id,
        channel_id: row.channel_id,
        name: row.name,
        avatar_url: row.avatar_url,
        webhook_type: row.webhook_type,
        token: String::new(),
        url: row.url,
        created_by: row.created_by,
        created_at: row.created_at,
    }
}

/// Ensure channel names are lowercase and start with #.
fn normalize_channel_name(name: &str) -> String {
    let name = name.to_lowercase();
    if name.starts_with('#') {
        name
    } else {
        format!("#{name}")
    }
}

fn channel_conversation_id(channel_id: &str) -> String {
    let mut id = String::with_capacity(8 + channel_id.len() * 2);
    id.push_str("channel:");
    for byte in channel_id.as_bytes() {
        use std::fmt::Write as _;
        write!(&mut id, "{byte:02X}").expect("writing to String cannot fail");
    }
    id
}

fn stable_irc_alias(name: &str, id: &str) -> String {
    let mut alias = String::new();
    let mut separator = false;
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            alias.push(character);
            separator = false;
        } else if !alias.is_empty() && !separator {
            alias.push('-');
            separator = true;
        }
    }
    while alias.ends_with('-') {
        alias.pop();
    }
    if alias.is_empty() {
        alias.push_str("server");
    }
    alias.truncate(20);
    let id_prefix: String = id.chars().take(8).collect();
    format!("{}-{id_prefix}", alias.trim_end_matches('-'))
}

fn parse_persisted_timestamp(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .map(|timestamp| timestamp.and_utc())
                .ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_interaction_response_rejects_unsafe_media_and_duplicate_controls() {
        let unsafe_embed = crate::engine::events::RichEmbedInfo {
            title: Some("Unsafe".into()),
            description: None,
            url: None,
            color: None,
            fields: None,
            footer: None,
            image_url: Some("javascript:alert(1)".into()),
            thumbnail_url: None,
            author: None,
            timestamp: None,
        };
        assert!(validate_rich_interaction_response(Some(&[unsafe_embed]), None).is_err());

        let button = crate::engine::events::MessageComponent::Button {
            custom_id: "same".into(),
            label: "Confirm".into(),
            style: "primary".into(),
            emoji: None,
            disabled: false,
        };
        let rows = [crate::engine::events::MessageComponent::ActionRow {
            components: vec![button.clone(), button],
        }];
        assert!(validate_rich_interaction_response(None, Some(&rows)).is_err());
    }

    #[test]
    fn rich_interaction_response_accepts_bounded_https_embed_and_controls() {
        let embed = crate::engine::events::RichEmbedInfo {
            title: Some("Result".into()),
            description: Some("Completed".into()),
            url: Some("https://example.test/result".into()),
            color: Some("#5865f2".into()),
            fields: None,
            footer: None,
            image_url: Some("https://example.test/image.png".into()),
            thumbnail_url: None,
            author: None,
            timestamp: None,
        };
        let rows = [crate::engine::events::MessageComponent::ActionRow {
            components: vec![crate::engine::events::MessageComponent::Button {
                custom_id: "confirm".into(),
                label: "Confirm".into(),
                style: "success".into(),
                emoji: None,
                disabled: false,
            }],
        }];
        validate_rich_interaction_response(Some(&[embed]), Some(&rows)).unwrap();
    }

    async fn moderation_engine_fixture() -> (ChatEngine, SqlitePool, ConnectionId, ConnectionId) {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username) VALUES \
             ('owner','owner'),('moderator','moderator'),('target','target')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('server','owner','owner'),('server','moderator','member'), \
             ('server','target','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO roles(id,server_id,name,position,permissions,is_default) VALUES \
             ('everyone','server','@everyone',0,?,1), \
             ('moderator-role','server','Moderator',10,?,0), \
             ('target-role','server','Target',1,0,0)",
        )
        .bind(DEFAULT_EVERYONE.bits() as i64)
        .bind(
            (Permissions::KICK_MEMBERS
                | Permissions::BAN_MEMBERS
                | Permissions::MANAGE_MESSAGES
                | Permissions::MANAGE_CHANNELS
                | Permissions::MANAGE_SERVER)
                .bits() as i64,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_roles(server_id,user_id,role_id) VALUES \
             ('server','moderator','moderator-role'),('server','target','target-role')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,channel_type) \
             VALUES('channel','server','#general','forum')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let moderator_actor = auth.issue_web_session("moderator").await.unwrap().1;
        let target_actor = auth.issue_web_session("target").await.unwrap().1;
        let engine = ChatEngine::new(pool.clone(), auth, "replay-secret", 4000, 100);
        engine.load_servers_from_db().await.unwrap();
        engine.load_channels_from_db().await.unwrap();
        let (moderator_session, _) = engine
            .connect(
                Some("moderator".into()),
                "moderator".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine
            .bind_authenticated_actor(moderator_session, moderator_actor)
            .unwrap();
        let (target_session, _) = engine
            .connect(
                Some("target".into()),
                "target".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine
            .bind_authenticated_actor(target_session, target_actor)
            .unwrap();
        engine
            .join_channel(target_session, "server", "#general")
            .await
            .unwrap();
        (engine, pool, moderator_session, target_session)
    }

    #[tokio::test]
    async fn persisted_button_invocation_is_authorized_and_routed_to_owning_bot() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot','bot',1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('server','bot','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) \
             VALUES('install','bot','server','moderator','commands','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let conversation_id: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content, \
                 conversation_id,conversation_sequence,components_json) \
             VALUES('response','server','channel','bot','bot','Choose',?,1,?)",
        )
        .bind(conversation_id)
        .bind(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO interactions(id,interaction_type,user_id,server_id,channel_id,data_json, \
                 application_user_id,expires_at,response_state,response_message_id) \
             VALUES('source','slash_command','target','server','channel','{}','bot', \
                 datetime('now','+5 minutes'),'responded','response')",
        )
        .execute(&pool)
        .await
        .unwrap();

        engine
            .invoke_message_component(target_session, "response", "confirm", &[])
            .await
            .unwrap();
        let invoked: (String, String, String, String) = sqlx::query_as(
            "SELECT interaction_type,user_id,application_user_id,data_json FROM interactions \
             WHERE id!='source'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(invoked.0, "button");
        assert_eq!(invoked.1, "target");
        assert_eq!(invoked.2, "bot");
        assert!(invoked.3.contains("confirm"));

        sqlx::query(
            "INSERT INTO interactions(id,interaction_type,user_id,server_id,channel_id,data_json, \
                 application_user_id,expires_at,response_state,ephemeral_response_json,response_expires_at) \
             VALUES('ephemeral-source','slash_command','target','server','channel','{}','bot', \
                 datetime('now','+5 minutes'),'responded',?,datetime('now','+5 minutes'))",
        )
        .bind(r#"{"content":"Choose","components":[{"type":"action_row","components":[{"type":"button","custom_id":"private-confirm","label":"Confirm"}]}],"ephemeral":true}"#)
        .execute(&pool)
        .await
        .unwrap();
        engine
            .invoke_message_component(
                target_session,
                "ephemeral:ephemeral-source",
                "private-confirm",
                &[],
            )
            .await
            .unwrap();
        let private_invocation: (String, String) = sqlx::query_as(
            "SELECT interaction_type,user_id FROM interactions \
             WHERE interaction_type='button' AND data_json LIKE '%private-confirm%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(private_invocation, ("button".into(), "target".into()));
        assert!(
            engine
                .invoke_message_component(
                    moderator_session,
                    "ephemeral:ephemeral-source",
                    "private-confirm",
                    &[],
                )
                .await
                .is_err()
        );

        sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now')")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            engine
                .invoke_message_component(target_session, "response", "confirm", &[])
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM interactions")
                .fetch_one(&pool)
                .await
                .unwrap(),
            4
        );
    }

    #[tokio::test]
    async fn component_invocation_revalidates_source_access_and_installation_after_admission_wait()
    {
        async fn wait_until_queued(engine: &ChatEngine, available_before: usize) {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let available = engine
                        .write_admission
                        .as_ref()
                        .unwrap()
                        .pending_available_permits_for_test();
                    if available < available_before {
                        return;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("component invocation should queue for write admission");
        }

        async fn prepare_component_source(pool: &SqlitePool) {
            sqlx::query("INSERT OR IGNORE INTO users(id,username,is_bot) VALUES('bot','bot',1)")
                .execute(pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT OR IGNORE INTO server_members(server_id,user_id,role) \
                 VALUES('server','bot','member')",
            )
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT OR REPLACE INTO bot_installations \
                 (id,bot_user_id,server_id,installed_by,granted_scopes,state,revoked_at) \
                 VALUES('install','bot','server','moderator','commands','active',NULL)",
            )
            .execute(pool)
            .await
            .unwrap();
            let conversation_id: String =
                sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                    .fetch_one(pool)
                    .await
                    .unwrap();
            sqlx::query(
                "INSERT OR REPLACE INTO messages \
                 (id,server_id,channel_id,sender_id,sender_nick,content,conversation_id, \
                  conversation_sequence,components_json,deleted_at) \
                 VALUES('response','server','channel','bot','bot','Choose',?,1,?,NULL)",
            )
            .bind(conversation_id)
            .bind(r#"[{"type":"action_row","components":[{"type":"button","custom_id":"confirm","label":"Confirm"}]}]"#)
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT OR REPLACE INTO interactions \
                 (id,interaction_type,user_id,server_id,channel_id,data_json,application_user_id, \
                  expires_at,response_state,response_message_id) \
                 VALUES('source','slash_command','target','server','channel','{}','bot', \
                  datetime('now','+5 minutes'),'responded','response')",
            )
            .execute(pool)
            .await
            .unwrap();
        }

        let (engine, pool, _moderator_session, target_session) = moderation_engine_fixture().await;
        prepare_component_source(&pool).await;
        let engine = std::sync::Arc::new(engine);

        for mutation in ["source_delete", "access_revoke", "uninstall"] {
            prepare_component_source(&pool).await;
            let admission = engine.write_admission.as_ref().unwrap();
            let held = admission.hold_active_capacity_for_test().await;
            let available_before = admission.pending_available_permits_for_test();
            let invocation = {
                let engine = engine.clone();
                tokio::spawn(async move {
                    engine
                        .invoke_message_component(target_session, "response", "confirm", &[])
                        .await
                })
            };
            wait_until_queued(&engine, available_before).await;

            match mutation {
                "source_delete" => {
                    sqlx::query(
                        "UPDATE messages SET deleted_at=datetime('now') WHERE id='response'",
                    )
                    .execute(&pool)
                    .await
                    .unwrap();
                }
                "access_revoke" => {
                    sqlx::query(
                        "DELETE FROM server_members WHERE server_id='server' AND user_id='target'",
                    )
                    .execute(&pool)
                    .await
                    .unwrap();
                }
                "uninstall" => {
                    sqlx::query(
                        "UPDATE bot_installations SET state='revoked',revoked_at=datetime('now') \
                         WHERE id='install'",
                    )
                    .execute(&pool)
                    .await
                    .unwrap();
                }
                _ => unreachable!(),
            }
            drop(held);
            let result = invocation.await.unwrap();
            assert!(
                result.is_err(),
                "{mutation} must invalidate the queued invocation"
            );
            let created: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM interactions WHERE interaction_type='button'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(created, 0, "{mutation} must not persist an interaction");

            if mutation == "access_revoke" {
                sqlx::query(
                    "INSERT INTO server_members(server_id,user_id,role) \
                     VALUES('server','target','member')",
                )
                .execute(&pool)
                .await
                .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn kick_commits_membership_and_audit_together_then_evicts_subscriptions() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        engine
            .kick_member(
                moderator_session,
                "server",
                "target",
                Some("documented reason"),
            )
            .await
            .unwrap();
        let state: (i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM server_members \
                 WHERE server_id='server' AND user_id='target'), \
                (SELECT count(*) FROM audit_log \
                 WHERE server_id='server' AND actor_id='moderator' \
                   AND action_type='member_kick' AND target_id='target')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 1));
        assert!(
            !engine
                .servers
                .get("server")
                .unwrap()
                .member_user_ids
                .contains("target")
        );
        assert!(
            !engine
                .channels
                .get("channel")
                .unwrap()
                .members
                .contains(&target_session)
        );
    }

    #[tokio::test]
    async fn kick_rolls_back_membership_when_audit_insert_fails() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        sqlx::query(
            "CREATE TRIGGER reject_kick_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='member_kick' BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            engine
                .kick_member(moderator_session, "server", "target", None)
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM server_members \
                 WHERE server_id='server' AND user_id='target'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert!(
            engine
                .servers
                .get("server")
                .unwrap()
                .member_user_ids
                .contains("target")
        );
        assert!(
            engine
                .channels
                .get("channel")
                .unwrap()
                .members
                .contains(&target_session)
        );
    }

    async fn insert_moderation_message(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO messages( \
                id,server_id,channel_id,sender_id,sender_nick,content, \
                conversation_id,conversation_sequence \
             ) VALUES( \
                ?,'server','channel','target','target','message', \
                (SELECT id FROM conversations WHERE channel_id='channel'), \
                (SELECT COALESCE(MAX(conversation_sequence),0)+1 FROM messages \
                 WHERE channel_id='channel') \
             )",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn bulk_delete_commits_tombstones_versions_events_outbox_and_audit() {
        let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
        insert_moderation_message(&pool, "message-1").await;
        insert_moderation_message(&pool, "message-2").await;

        engine
            .bulk_delete_messages(
                moderator_session,
                "server",
                "#general",
                vec!["message-1".into(), "message-2".into()],
            )
            .await
            .unwrap();

        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM messages WHERE id IN ('message-1','message-2') \
                 AND deleted_at IS NOT NULL AND entity_version=2), \
                (SELECT count(*) FROM entity_versions WHERE entity_type='message' \
                 AND entity_id IN ('message-1','message-2') AND version=2), \
                (SELECT count(*) FROM event_log e JOIN delivery_outbox o USING(event_sequence) \
                 WHERE e.event_kind='message_deleted' \
                 AND e.entity_id IN ('message-1','message-2')), \
                (SELECT count(*) FROM audit_log WHERE action_type='message_bulk_delete' \
                 AND actor_id='moderator' AND target_id='channel')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (2, 2, 2, 1));
    }

    #[tokio::test]
    async fn bulk_delete_rolls_back_all_tombstones_when_audit_fails() {
        let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
        insert_moderation_message(&pool, "message-1").await;
        insert_moderation_message(&pool, "message-2").await;
        sqlx::query(
            "CREATE TRIGGER reject_bulk_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='message_bulk_delete' \
             BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(
            engine
                .bulk_delete_messages(
                    moderator_session,
                    "server",
                    "#general",
                    vec!["message-1".into(), "message-2".into()],
                )
                .await
                .is_err()
        );
        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM messages WHERE id IN ('message-1','message-2') \
                 AND deleted_at IS NULL AND entity_version=1), \
                (SELECT count(*) FROM entity_versions WHERE entity_type='message' \
                 AND entity_id IN ('message-1','message-2')), \
                (SELECT count(*) FROM event_log WHERE event_kind='message_deleted' \
                 AND entity_id IN ('message-1','message-2')), \
                (SELECT count(*) FROM audit_log WHERE action_type='message_bulk_delete')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (2, 0, 0, 0));
    }

    #[tokio::test]
    async fn ban_commits_membership_ban_and_audit_then_evicts_subscriptions() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        engine
            .ban_member(
                moderator_session,
                "server",
                "target",
                Some("documented reason"),
                0,
            )
            .await
            .unwrap();
        let state: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM server_members \
                 WHERE server_id='server' AND user_id='target'), \
                (SELECT count(*) FROM bans \
                 WHERE server_id='server' AND user_id='target' AND banned_by='moderator'), \
                (SELECT count(*) FROM audit_log \
                 WHERE action_type='member_ban' AND actor_id='moderator' AND target_id='target')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 1, 1));
        assert!(
            !engine
                .channels
                .get("channel")
                .unwrap()
                .members
                .contains(&target_session)
        );
    }

    #[tokio::test]
    async fn ban_rolls_back_membership_and_ban_when_audit_fails() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        sqlx::query(
            "CREATE TRIGGER reject_ban_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='member_ban' BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            engine
                .ban_member(moderator_session, "server", "target", None, 0)
                .await
                .is_err()
        );
        let state: (i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM server_members \
                 WHERE server_id='server' AND user_id='target'), \
                (SELECT count(*) FROM bans WHERE server_id='server' AND user_id='target')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (1, 0));
        assert!(
            engine
                .channels
                .get("channel")
                .unwrap()
                .members
                .contains(&target_session)
        );
    }

    #[tokio::test]
    async fn ban_cleanup_advances_in_canonical_restart_safe_batches() {
        let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
        for index in 0..102 {
            insert_moderation_message(&pool, &format!("ban-message-{index:03}")).await;
        }
        let conversation_id: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO attachments( \
                id,uploader_id,message_id,filename,original_filename,content_type,file_size, \
                conversation_id,media_state,storage_backend,storage_key,reserved_bytes \
             ) VALUES('ban-attachment','target','ban-message-000','file','file', \
                'text/plain',4,?,'attached','local','ban-key',4)",
        )
        .bind(&conversation_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('announcement-server','Announcements','moderator')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('announcement-server','moderator','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('announcement-channel','announcement-server','#announcements')")
            .execute(&pool)
            .await
            .unwrap();
        let announcement_conversation: String = sqlx::query_scalar(
            "SELECT id FROM conversations WHERE channel_id='announcement-channel'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages( \
                id,server_id,channel_id,sender_id,sender_nick,content, \
                conversation_id,conversation_sequence \
             ) VALUES('announcement-copy','announcement-server','announcement-channel', \
                'target','target','copy',?,1)",
        )
        .bind(&announcement_conversation)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhooks( \
                id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
             ) VALUES('announcement-delete-hook','announcement-server','announcement-channel', \
                'Announcement Hook','outgoing','announcement-delete-token', \
                'https://example.com/announcement-hook','moderator','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhook_events(id,webhook_id,event_type) \
             VALUES('announcement-delete-subscription','announcement-delete-hook','message_delete')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO announcement_publications( \
                id,follow_id,source_message_id,target_message_id,source_version,state \
             ) VALUES('publication','historical-follow','ban-message-000', \
                'announcement-copy',1,'published')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhooks( \
                id,server_id,channel_id,name,webhook_type,token,url,created_by,credential_state \
             ) VALUES('delete-hook','server','channel','Hook','outgoing','delete-token', \
                'https://example.com/hook','moderator','active')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO webhook_events(id,webhook_id,event_type) \
             VALUES('delete-subscription','delete-hook','message_delete')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE conversations SET next_message_sequence=( \
                SELECT COALESCE(MAX(conversation_sequence),0)+1 FROM messages \
                WHERE conversation_id=conversations.id \
             ) WHERE id=?",
        )
        .bind(&conversation_id)
        .execute(&pool)
        .await
        .unwrap();
        engine
            .ban_member(moderator_session, "server", "target", None, 7)
            .await
            .unwrap();
        engine
            .unban_member(moderator_session, "server", "target")
            .await
            .unwrap();
        sqlx::query("DELETE FROM messages WHERE id='ban-message-101'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages( \
                id,server_id,channel_id,sender_id,sender_nick,content, \
                conversation_id,conversation_sequence \
             ) SELECT 'post-unban-message','server','channel','target','target','new', \
                id,next_message_sequence FROM conversations WHERE id=?",
        )
        .bind(&conversation_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE conversations SET next_message_sequence=next_message_sequence+1 WHERE id=?",
        )
        .bind(&conversation_id)
        .execute(&pool)
        .await
        .unwrap();

        let scheduled: (String, i64, i64) = sqlx::query_as(
            "SELECT state,deleted_count, \
                (SELECT count(*) FROM messages WHERE server_id='server' \
                 AND sender_id='target' AND deleted_at IS NULL) \
             FROM moderation_cleanup_jobs WHERE server_id='server' AND user_id='target'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(scheduled, ("pending".into(), 0, 102));

        assert_eq!(
            engine.process_moderation_cleanup_batch().await.unwrap(),
            100
        );
        let first: (String, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT state,deleted_count, \
                (SELECT count(*) FROM messages WHERE server_id='server' \
                 AND sender_id='target' AND deleted_at IS NOT NULL), \
                (SELECT count(*) FROM entity_versions WHERE entity_type='message' \
                 AND entity_id LIKE 'ban-message-%' AND version=2), \
                (SELECT count(*) FROM event_log e JOIN delivery_outbox o USING(event_sequence) \
                 WHERE e.event_kind='message_deleted' AND e.entity_id LIKE 'ban-message-%') \
             FROM moderation_cleanup_jobs WHERE server_id='server' AND user_id='target'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(first, ("pending".into(), 100, 100, 100, 100));

        assert_eq!(engine.process_moderation_cleanup_batch().await.unwrap(), 1);
        let completed: (String, i64, i64) = sqlx::query_as(
            "SELECT state,deleted_count, \
                (SELECT count(*) FROM messages WHERE server_id='server' \
                 AND sender_id='target' AND deleted_at IS NOT NULL) \
             FROM moderation_cleanup_jobs WHERE server_id='server' AND user_id='target'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(completed, ("completed".into(), 101, 101));
        let canonical_effects: (String, i64, String, i64) = sqlx::query_as(
            "SELECT media_state, \
                (SELECT deleted_at IS NOT NULL FROM messages WHERE id='announcement-copy'), \
                (SELECT state FROM announcement_publications WHERE id='publication'), \
                (SELECT count(*) FROM webhook_deliveries WHERE event_type='message_delete') \
             FROM attachments WHERE id='ban-attachment'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            canonical_effects,
            ("deleting".into(), 1, "deleted".into(), 102)
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM messages WHERE id='post-unban-message' AND deleted_at IS NULL"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(engine.process_moderation_cleanup_batch().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn timeout_rolls_back_member_state_when_audit_fails() {
        let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
        sqlx::query(
            "CREATE TRIGGER reject_timeout_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='member_timeout' BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let until = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        assert!(
            engine
                .timeout_member(moderator_session, "server", "target", Some(&until), None)
                .await
                .is_err()
        );
        let state: (Option<String>, i64) = sqlx::query_as(
            "SELECT timeout_until, \
                (SELECT count(*) FROM audit_log WHERE action_type='member_timeout') \
             FROM server_members WHERE server_id='server' AND user_id='target'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (None, 0));
    }

    #[tokio::test]
    async fn automod_crud_commits_each_rule_change_with_audit() {
        let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
        engine
            .create_automod_rule(
                moderator_session,
                &CreateAutomodRuleRequest {
                    server_id: "server",
                    name: "keywords",
                    rule_type: "keyword",
                    config: r#"{"words":["blocked"]}"#,
                    action_type: "delete",
                    timeout_duration_seconds: None,
                },
            )
            .await
            .unwrap();
        let rule_id: String =
            sqlx::query_scalar("SELECT id FROM automod_rules WHERE server_id='server'")
                .fetch_one(&pool)
                .await
                .unwrap();
        engine
            .update_automod_rule(
                moderator_session,
                &UpdateAutomodRuleRequest {
                    rule_id: &rule_id,
                    server_id: "server",
                    name: "mentions",
                    enabled: false,
                    config: r#"{"words":["blocked","second"]}"#,
                    action_type: "flag",
                    timeout_duration_seconds: None,
                },
            )
            .await
            .unwrap();
        engine
            .delete_automod_rule(moderator_session, "server", &rule_id)
            .await
            .unwrap();

        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM automod_rules WHERE server_id='server'), \
                (SELECT count(*) FROM audit_log WHERE action_type='automod_rule_create'), \
                (SELECT count(*) FROM audit_log WHERE action_type='automod_rule_update'), \
                (SELECT count(*) FROM audit_log WHERE action_type='automod_rule_delete')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 1, 1, 1));
    }

    #[tokio::test]
    async fn automod_create_rolls_back_rule_when_audit_fails() {
        let (engine, pool, moderator_session, _) = moderation_engine_fixture().await;
        sqlx::query(
            "CREATE TRIGGER reject_automod_create_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='automod_rule_create' \
             BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            engine
                .create_automod_rule(
                    moderator_session,
                    &CreateAutomodRuleRequest {
                        server_id: "server",
                        name: "keywords",
                        rule_type: "keyword",
                        config: r#"{"words":["blocked"]}"#,
                        action_type: "delete",
                        timeout_duration_seconds: None,
                    },
                )
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM automod_rules WHERE server_id='server'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn forum_tags_enforce_parent_ownership_and_moderated_authority() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        insert_moderation_message(&pool, "thread-parent").await;
        engine
            .create_thread(
                target_session,
                "server",
                "#general",
                "topic",
                "thread-parent",
                false,
            )
            .await
            .unwrap();
        let thread_id: String =
            sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT thread_creator_user_id FROM channels WHERE id=?"
            )
            .bind(&thread_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .as_deref(),
            Some("target")
        );
        engine
            .create_forum_tag(
                moderator_session,
                "server",
                "#general",
                "ordinary",
                None,
                false,
            )
            .await
            .unwrap();
        engine
            .create_forum_tag(
                moderator_session,
                "server",
                "#general",
                "moderated",
                None,
                true,
            )
            .await
            .unwrap();
        let ordinary: String = sqlx::query_scalar(
            "SELECT id FROM forum_tags WHERE channel_id='channel' AND name='ordinary'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let moderated: String = sqlx::query_scalar(
            "SELECT id FROM forum_tags WHERE channel_id='channel' AND name='moderated'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        engine
            .set_thread_tags(target_session, "server", &thread_id, vec![ordinary.clone()])
            .await
            .unwrap();
        assert!(
            engine
                .set_thread_tags(
                    target_session,
                    "server",
                    &thread_id,
                    vec![moderated.clone()],
                )
                .await
                .is_err()
        );
        let selected: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id=?")
                .bind(&thread_id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(selected, vec![ordinary]);
        engine
            .set_thread_tags(
                moderator_session,
                "server",
                &thread_id,
                vec![moderated.clone()],
            )
            .await
            .unwrap();
        assert!(
            engine
                .set_thread_tags(target_session, "server", &thread_id, Vec::new())
                .await
                .is_err()
        );
        engine
            .delete_forum_tag(moderator_session, "server", "#general", &moderated)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM thread_tags WHERE thread_id=?")
                .bind(&thread_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        let durable: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT thread_tags_version, \
                (SELECT count(*) FROM event_log WHERE entity_type='thread_tags' AND entity_id=?), \
                (SELECT count(*) FROM delivery_outbox o JOIN event_log e USING(event_sequence) \
                 WHERE e.entity_type='thread_tags' AND e.entity_id=?), \
                (SELECT count(*) FROM audit_log WHERE action_type='thread_tags_update' \
                 AND target_id=?) \
             FROM channels WHERE id=?",
        )
        .bind(&thread_id)
        .bind(&thread_id)
        .bind(&thread_id)
        .bind(&thread_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(durable, (4, 3, 3, 3));
        let projection = engine.channels.get(&thread_id).unwrap();
        assert_eq!(projection.thread_tags_version, 4);
        assert!(projection.thread_tag_ids.is_empty());
    }

    #[tokio::test]
    async fn thread_tag_audit_failure_rolls_back_selection_version_and_event() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        insert_moderation_message(&pool, "thread-parent").await;
        engine
            .create_thread(
                target_session,
                "server",
                "#general",
                "topic",
                "thread-parent",
                false,
            )
            .await
            .unwrap();
        let thread_id: String =
            sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        engine
            .create_forum_tag(
                moderator_session,
                "server",
                "#general",
                "ordinary",
                None,
                false,
            )
            .await
            .unwrap();
        let tag_id: String =
            sqlx::query_scalar("SELECT id FROM forum_tags WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_thread_tag_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='thread_tags_update' \
             BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(
            engine
                .set_thread_tags(target_session, "server", &thread_id, vec![tag_id])
                .await
                .is_err()
        );
        let state: (i64, i64, i64) = sqlx::query_as(
            "SELECT thread_tags_version, \
                (SELECT count(*) FROM thread_tags WHERE thread_id=?), \
                (SELECT count(*) FROM event_log WHERE entity_type='thread_tags' AND entity_id=?) \
             FROM channels WHERE id=?",
        )
        .bind(&thread_id)
        .bind(&thread_id)
        .bind(&thread_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (1, 0, 0));
        assert_eq!(
            engine.channels.get(&thread_id).unwrap().thread_tags_version,
            1
        );
    }

    #[tokio::test]
    async fn legacy_unknown_thread_creator_has_no_guessed_tag_authority() {
        let (engine, pool, moderator_session, target_session) = moderation_engine_fixture().await;
        insert_moderation_message(&pool, "thread-parent").await;
        engine
            .create_thread(
                target_session,
                "server",
                "#general",
                "topic",
                "thread-parent",
                false,
            )
            .await
            .unwrap();
        let thread_id: String =
            sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query("UPDATE channels SET thread_creator_user_id=NULL WHERE id=?")
            .bind(&thread_id)
            .execute(&pool)
            .await
            .unwrap();
        engine
            .create_forum_tag(
                moderator_session,
                "server",
                "#general",
                "ordinary",
                None,
                false,
            )
            .await
            .unwrap();
        let tag_id: String =
            sqlx::query_scalar("SELECT id FROM forum_tags WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            engine
                .set_thread_tags(target_session, "server", &thread_id, vec![tag_id.clone()])
                .await
                .is_err()
        );
        engine
            .set_thread_tags(moderator_session, "server", &thread_id, vec![tag_id])
            .await
            .unwrap();
    }

    #[test]
    fn test_normalize_channel_name() {
        assert_eq!(normalize_channel_name("#General"), "#general");
        assert_eq!(normalize_channel_name("general"), "#general");
        assert_eq!(normalize_channel_name("#rust"), "#rust");
    }

    #[test]
    fn irc_alias_is_stable_bounded_and_not_display_name_routing() {
        assert_eq!(
            stable_irc_alias("My Long Server Name!", "12345678-rest"),
            "my-long-server-name-12345678"
        );
        assert_eq!(stable_irc_alias("🦇", "abcdef"), "server-abcdef");
        assert_eq!(
            stable_irc_alias("Concord", "🦇archive"),
            "concord-🦇archive"
        );
    }

    #[test]
    fn persisted_timestamps_accept_current_and_legacy_sqlite_forms_exactly() {
        assert_eq!(
            parse_persisted_timestamp("2024-01-02 03:04:05.123456")
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "2024-01-02T03:04:05.123456Z"
        );
        assert_eq!(
            parse_persisted_timestamp("2024-01-02T03:04:05.123456+00:00")
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "2024-01-02T03:04:05.123456Z"
        );
        assert!(parse_persisted_timestamp("not-a-timestamp").is_none());
    }

    #[tokio::test]
    async fn history_reload_and_snapshot_preserve_legacy_and_current_timestamps() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
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
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let conversation_id: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let legacy_id = format!(" historical:旧消息/{} ", "界".repeat(512));
        let current_id = "10000000-0000-0000-0000-000000000002";
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at,edited_at,conversation_id,conversation_sequence,content_format) VALUES \
             (?, 'server','channel','owner','owner','legacy','2024-01-02 03:04:05.123456','2024-01-02 04:05:06.654321',?,1,'plain'), \
             (?, 'server','channel','owner','owner','current','2024-01-03T03:04:05.123456+00:00',NULL,?,2,'plain')",
        )
        .bind(&legacy_id)
        .bind(&conversation_id)
        .bind(current_id)
        .bind(&conversation_id)
        .execute(&pool)
        .await
        .unwrap();

        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("owner").await.unwrap().1;
        let engine = ChatEngine::new(pool.clone(), auth, "replay-secret", 4000, 100);
        engine.load_servers_from_db().await.unwrap();
        engine.load_channels_from_db().await.unwrap();
        let first = engine
            .fetch_history("server", "#general", None, 50, &actor)
            .await
            .unwrap()
            .0;
        let first_wire = serde_json::to_value(&first).unwrap();
        assert_eq!(first_wire[0]["id"], current_id);
        assert_eq!(first_wire[0]["timestamp"], "2024-01-03T03:04:05.123456Z");
        assert_eq!(first_wire[1]["id"], legacy_id);
        assert_eq!(first_wire[1]["timestamp"], "2024-01-02T03:04:05.123456Z");
        assert_eq!(first_wire[1]["edited_at"], "2024-01-02T04:05:06.654321Z");

        let snapshot = engine
            .replay_service()
            .snapshot(&actor, std::slice::from_ref(&conversation_id))
            .await
            .unwrap();
        assert_eq!(snapshot.messages[0].message_id, legacy_id);
        assert_eq!(
            snapshot.messages[0].created_at,
            "2024-01-02 03:04:05.123456"
        );
        assert_eq!(snapshot.messages[1].message_id, current_id);
        assert_eq!(
            snapshot.messages[1].created_at,
            "2024-01-03T03:04:05.123456+00:00"
        );

        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("owner").await.unwrap().1;
        let reloaded = ChatEngine::new(pool, auth, "replay-secret", 4000, 100);
        reloaded.load_servers_from_db().await.unwrap();
        reloaded.load_channels_from_db().await.unwrap();
        let second = reloaded
            .fetch_history("server", "#general", None, 50, &actor)
            .await
            .unwrap()
            .0;
        assert_eq!(serde_json::to_value(second).unwrap(), first_wire);
    }

    #[tokio::test]
    async fn search_continuation_binds_query_and_restarts_after_authority_change() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('server','owner','owner'),('server','member','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES \
             ('everyone','server','@everyone',?,1)",
        )
        .bind((Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY).bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES \
             ('public','server','#public'),('private','server','#private')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
             ('public-old','server','public','owner','owner','needle','2026-09-01T00:00:00Z'), \
             ('private-new','server','private','owner','owner','needle','2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let actor = auth.issue_web_session("member").await.unwrap().1;
        let engine = ChatEngine::new(pool.clone(), auth, "search-secret", 4000, 100);
        engine.load_servers_from_db().await.unwrap();
        engine.load_channels_from_db().await.unwrap();
        let first = engine
            .search_messages(
                &actor,
                SearchMessagesRequest {
                    server_id: "server",
                    query: "needle",
                    channel_name: None,
                    limit: 1,
                    offset: 0,
                    continuation: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(first.results[0].id, "private-new");
        let continuation = first.next_continuation.unwrap();
        assert!(matches!(
            engine
                .search_messages(
                    &actor,
                    SearchMessagesRequest {
                        server_id: "server",
                        query: "different query",
                        channel_name: None,
                        limit: 1,
                        offset: 0,
                        continuation: Some(&continuation),
                    },
                )
                .await,
            Err(SearchError::InvalidContinuation)
        ));
        let second_credential = engine
            .auth
            .get()
            .unwrap()
            .issue_web_session("member")
            .await
            .unwrap()
            .1;
        assert!(matches!(
            engine
                .search_messages(
                    &second_credential,
                    SearchMessagesRequest {
                        server_id: "server",
                        query: "needle",
                        channel_name: None,
                        limit: 1,
                        offset: 0,
                        continuation: Some(&continuation),
                    },
                )
                .await,
            Err(SearchError::InvalidContinuation)
        ));

        sqlx::query(
            "INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) \
             VALUES('deny-private','private','role','everyone',?)",
        )
        .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        let restarted = engine
            .search_messages(
                &actor,
                SearchMessagesRequest {
                    server_id: "server",
                    query: "needle",
                    channel_name: None,
                    limit: 1,
                    offset: 0,
                    continuation: Some(&continuation),
                },
            )
            .await
            .unwrap();
        assert!(restarted.restarted);
        assert_eq!(restarted.offset, 0);
        assert_eq!(restarted.total_count, 1);
        assert_eq!(restarted.results[0].id, "public-old");
    }

    /// Helper: create engine with a default server in memory (no DB).
    fn setup_engine() -> ChatEngine {
        let engine = ChatEngine::test_harness(4000, 100);
        let mut state = ServerState::new(
            DEFAULT_SERVER_ID.to_string(),
            "Concord".to_string(),
            "system".to_string(),
            None,
        );
        for (id, name) in [("test-general", "#general"), ("test-rust", "#rust")] {
            state.channel_ids.insert(id.to_string());
            engine.channels.insert(
                id.to_string(),
                ChannelState::new(
                    id.to_string(),
                    DEFAULT_SERVER_ID.to_string(),
                    name.to_string(),
                ),
            );
            engine.channel_name_index.insert(
                (DEFAULT_SERVER_ID.to_string(), name.to_string()),
                id.to_string(),
            );
        }
        engine.servers.insert(DEFAULT_SERVER_ID.to_string(), state);
        engine
    }

    #[tokio::test]
    async fn test_connect_and_disconnect() {
        let engine = setup_engine();

        let (session_id, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        assert!(!engine.is_nick_available("alice"));

        engine.disconnect(session_id);
        assert!(engine.is_nick_available("alice"));
    }

    #[tokio::test]
    async fn test_same_user_can_hold_multiple_sessions_with_one_nickname() {
        let engine = setup_engine();

        let (sid1, _rx1) = engine
            .connect(
                Some("user-1".into()),
                "alice".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        let (sid2, _rx2) = engine
            .connect(Some("user-1".into()), "alice".into(), Protocol::Irc, None)
            .unwrap();

        assert!(engine.get_session(sid1).is_some());
        assert!(engine.get_session(sid2).is_some());
        assert_eq!(engine.user_connections.get("user-1").unwrap().len(), 2);
        engine.disconnect(sid2);
        assert_eq!(engine.get_session_id_by_nick("alice"), Some(sid1));
        assert_eq!(engine.user_connections.get("user-1").unwrap().len(), 1);
        engine.disconnect(sid1);
        assert!(engine.user_connections.get("user-1").is_none());
        assert!(engine.is_nick_available("alice"));
    }

    #[tokio::test]
    async fn disconnecting_one_user_transport_does_not_emit_a_false_quit() {
        let engine = setup_engine();
        let (sid1, _rx1) = engine
            .connect(
                Some("user-1".into()),
                "alice".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        let (sid2, _rx2) = engine
            .connect(Some("user-1".into()), "alice".into(), Protocol::Irc, None)
            .unwrap();
        let (observer, mut observer_rx) = engine
            .connect(None, "observer".into(), Protocol::WebSocket, None)
            .unwrap();
        let mut channel = ChannelState::new(
            "channel".into(),
            DEFAULT_SERVER_ID.into(),
            "#general".into(),
        );
        channel.members.insert(sid1);
        channel.members.insert(sid2);
        channel.members.insert(observer);
        engine.channels.insert("channel".into(), channel);

        engine.disconnect(sid1);

        assert!(observer_rx.try_recv().is_err());
        assert_eq!(engine.user_connections.get("user-1").unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_credential_tracks_all_connections_until_final_disconnect() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('user-1','alice')")
            .execute(&pool)
            .await
            .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("user-1").await.unwrap();
        let credential_id = actor.credential_id().clone();
        let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);
        let (sid1, _) = engine
            .connect(
                Some("user-1".into()),
                "alice".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        let (sid2, _) = engine
            .connect(Some("user-1".into()), "alice".into(), Protocol::Irc, None)
            .unwrap();

        engine
            .bind_authenticated_actor(sid1, actor.clone())
            .unwrap();
        engine.bind_authenticated_actor(sid2, actor).unwrap();
        assert_eq!(
            engine
                .credential_connections
                .get(&credential_id)
                .unwrap()
                .len(),
            2
        );

        engine.disconnect(sid1);
        assert_eq!(
            engine
                .credential_connections
                .get(&credential_id)
                .unwrap()
                .len(),
            1
        );
        engine.disconnect(sid2);
        assert!(engine.credential_connections.get(&credential_id).is_none());
    }

    #[tokio::test]
    async fn nickname_remains_exclusive_across_users() {
        let engine = setup_engine();
        engine
            .connect(
                Some("user-1".into()),
                "alice".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        assert!(
            engine
                .connect(Some("user-2".into()), "alice".into(), Protocol::Irc, None)
                .is_err()
        );
    }

    #[tokio::test]
    async fn rfc1459_equivalent_nicknames_are_one_identity_slot() {
        let engine = setup_engine();
        engine
            .connect(Some("user-1".into()), "Nick[".into(), Protocol::Irc, None)
            .unwrap();
        assert!(
            engine
                .connect(Some("user-2".into()), "nICK{".into(), Protocol::Irc, None)
                .is_err()
        );
    }

    #[tokio::test]
    async fn web_display_name_is_not_limited_by_irc_nickname_width() {
        let engine = setup_engine();
        let display_name = format!("{}-example.social", "long-handle".repeat(4));
        assert!(
            display_name.len() > super::validation::MAX_NICKNAME_LENGTH
                && engine
                    .connect(
                        Some("user-1".into()),
                        display_name,
                        Protocol::WebSocket,
                        None,
                    )
                    .is_ok()
        );
    }

    #[tokio::test]
    async fn profile_projection_requires_self_or_shared_server() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('viewer','viewer'),('shared','shared'),('outsider','outsider')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','viewer')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','viewer','owner'),('server','shared','member')")
            .execute(&pool).await.unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("viewer").await.unwrap();
        let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);

        assert!(engine.get_user_profile(&actor, "viewer").await.is_ok());
        assert!(engine.get_user_profile(&actor, "shared").await.is_ok());
        assert!(engine.get_user_profile(&actor, "outsider").await.is_err());
    }

    #[tokio::test]
    async fn atproto_profile_sync_updates_stable_identity_and_live_projection() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username) VALUES('local-user','alice'),('viewer','viewer')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','local-user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('server','local-user','owner'),('server','viewer','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) \
             VALUES('at-account','local-user','atproto','did:plc:alice')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let sync_actor = auth.issue_web_session("local-user").await.unwrap().1;
        let viewer_actor = auth.issue_web_session("viewer").await.unwrap().1;
        let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);
        engine.load_servers_from_db().await.unwrap();
        engine.load_channels_from_db().await.unwrap();
        let (viewer_session, mut events) = engine
            .connect(
                Some("viewer".into()),
                "viewer".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine
            .bind_authenticated_actor(viewer_session, viewer_actor)
            .unwrap();
        engine
            .channels
            .get_mut("channel")
            .unwrap()
            .members
            .insert(viewer_session);

        let did = engine
            .verified_atproto_profile_did(&sync_actor)
            .await
            .unwrap();
        assert_eq!(did, "did:plc:alice");
        let input = super::super::profile_sync::BlueskyProfileSyncInput {
            did: "did:plc:alice",
            handle: "alice.test",
            display_name: Some("Alice"),
            description: Some("Synced biography"),
            avatar: Some("https://cdn.test/avatar.jpg"),
            banner: Some("https://cdn.test/banner.jpg"),
            followers_count: 4,
            follows_count: 3,
        };
        let updated = engine
            .apply_atproto_profile_sync(&sync_actor, &did, &input)
            .await
            .unwrap();
        assert_eq!(updated.user_id, "local-user");

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            ChatEvent::UserProfile { profile }
                if profile.user_id == "local-user"
                    && profile.bio.as_deref() == Some("Synced biography")
        ));
    }

    #[tokio::test]
    async fn presence_projection_uses_durable_identity_hides_invisible_and_fails_closed() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username,avatar_url) VALUES \
             ('viewer','Viewer',NULL),('会員識別子','Durable Name','durable-avatar')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','viewer')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role,nickname,avatar_url) VALUES \
             ('server','viewer','owner',NULL,NULL), \
             ('server','会員識別子','member','Server Name','server-avatar')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_presence(user_id,status,requested_status,custom_status,status_emoji) \
             VALUES('会員識別子','invisible','invisible','secret','secret-emoji')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, viewer_actor) = auth.issue_web_session("viewer").await.unwrap();
        let engine = ChatEngine::new(pool.clone(), auth, "test-replay-secret", 4000, 100);
        let (viewer_session, _) = engine
            .connect(
                Some("viewer".into()),
                "Viewer Live".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine
            .bind_authenticated_actor(viewer_session, viewer_actor)
            .unwrap();
        engine
            .connect(
                Some("会員識別子".into()),
                "Transient Live Name".into(),
                Protocol::WebSocket,
                Some("transient-avatar".into()),
            )
            .unwrap();

        let projected = engine
            .get_server_presences(viewer_session, "server")
            .await
            .unwrap();
        let invisible = projected
            .iter()
            .find(|presence| presence.user_id == "会員識別子")
            .unwrap();
        assert_eq!(invisible.nickname, "Server Name");
        assert_eq!(invisible.avatar_url.as_deref(), Some("server-avatar"));
        assert_eq!(invisible.status, "offline");
        assert_eq!(invisible.custom_status, None);
        assert_eq!(invisible.status_emoji, None);

        pool.close().await;
        assert!(matches!(
            engine.get_server_presences(viewer_session, "server").await,
            Err(error) if error.starts_with("DEPENDENCY_UNAVAILABLE:")
        ));
    }

    #[tokio::test]
    async fn queued_server_response_is_rejected_after_ban() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('viewer','viewer')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','viewer','member')")
            .execute(&pool).await.unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("viewer").await.unwrap();
        let engine = ChatEngine::new(pool.clone(), auth, "test-replay-secret", 4000, 100);
        let (session_id, mut receiver) = engine
            .connect(
                Some("viewer".into()),
                "viewer".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine
            .bind_authenticated_actor(session_id, actor.clone())
            .unwrap();
        let session = engine.get_session(session_id).unwrap();
        assert!(session.send_guarded(
            ChatEvent::ChannelList {
                server_id: "server".into(),
                channels: Vec::new(),
            },
            Some(super::super::user_session::DeliveryGuard::ServerMembership(
                vec!["server".into(),]
            )),
        ));
        receiver.recv().await.unwrap();
        let guard = session
            .take_delivery_guard()
            .expect("server response is guarded");
        assert!(engine.delivery_guard_is_current(&actor, &guard).await);

        sqlx::query("INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','viewer','owner')")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!engine.delivery_guard_is_current(&actor, &guard).await);
    }

    #[tokio::test]
    async fn private_thread_creation_does_not_enqueue_for_unrelated_parent_viewers() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('viewer','viewer')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','viewer','member')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
            .bind(DEFAULT_EVERYONE.bits() as i64)
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('parent','server','#parent')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,conversation_id,conversation_sequence) VALUES('parent-message','server','parent','owner','owner','parent',(SELECT id FROM conversations WHERE channel_id='parent'),1)")
            .execute(&pool).await.unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, owner_actor) = auth.issue_web_session("owner").await.unwrap();
        let (_, viewer_actor) = auth.issue_web_session("viewer").await.unwrap();
        let engine = ChatEngine::new(pool, auth, "test-replay-secret", 4000, 100);
        engine.load_servers_from_db().await.unwrap();
        engine.load_channels_from_db().await.unwrap();
        let (owner_session, mut owner_rx) = engine
            .connect(
                Some("owner".into()),
                "owner".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        let (viewer_session, mut viewer_rx) = engine
            .connect(
                Some("viewer".into()),
                "viewer".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine
            .bind_authenticated_actor(owner_session, owner_actor)
            .unwrap();
        engine
            .bind_authenticated_actor(viewer_session, viewer_actor)
            .unwrap();
        engine
            .join_channel(owner_session, "server", "#parent")
            .await
            .unwrap();
        engine
            .join_channel(viewer_session, "server", "#parent")
            .await
            .unwrap();
        while owner_rx.try_recv().is_ok() {}
        while viewer_rx.try_recv().is_ok() {}

        engine
            .create_thread(
                owner_session,
                "server",
                "#parent",
                "private",
                "parent-message",
                true,
            )
            .await
            .unwrap();
        assert!(matches!(
            owner_rx.try_recv(),
            Ok(ChatEvent::ThreadCreate { .. })
        ));
        assert!(viewer_rx.try_recv().is_err());
        assert!(engine.get_session(viewer_session).is_some());

        let pool = engine.get_db().unwrap();
        let thread_id: String =
            sqlx::query_scalar("SELECT id FROM channels WHERE parent_channel_id='parent'")
                .fetch_one(&pool)
                .await
                .unwrap();
        engine
            .archive_thread(owner_session, "server", &thread_id)
            .await
            .unwrap();
        engine
            .unarchive_thread(owner_session, "server", &thread_id)
            .await
            .unwrap();
        let state: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT archived,thread_state_version, \
                (SELECT count(*) FROM event_log WHERE entity_type='thread_state' AND entity_id=?), \
                (SELECT count(*) FROM delivery_outbox o JOIN event_log e USING(event_sequence) \
                 WHERE e.entity_type='thread_state' AND e.entity_id=?) \
             FROM channels WHERE id=?",
        )
        .bind(&thread_id)
        .bind(&thread_id)
        .bind(&thread_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (0, 3, 2, 2));
        assert!(!engine.channels.get(&thread_id).unwrap().archived);
        engine.apply_thread_state_projection(&thread_id, true, 2);
        let projected = engine.channels.get(&thread_id).unwrap();
        assert!(!projected.archived);
        assert_eq!(projected.thread_state_version, 3);
    }

    #[tokio::test]
    async fn queued_channel_response_is_rejected_after_read_history_revocation() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('viewer','viewer')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','viewer','member')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
            .bind((Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY).bits() as i64)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#private')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let (_, actor) = auth.issue_web_session("viewer").await.unwrap();
        let engine = ChatEngine::new(pool.clone(), auth, "replay-secret", 4000, 100);
        let guard = super::super::user_session::DeliveryGuard::ChannelActions(vec![(
            "channel".into(),
            super::super::authorization::ChannelAction::ReadHistory,
        )]);
        assert!(engine.delivery_guard_is_current(&actor, &guard).await);

        sqlx::query("UPDATE roles SET permissions=? WHERE id='everyone'")
            .bind(Permissions::VIEW_CHANNELS.bits() as i64)
            .execute(&pool)
            .await
            .unwrap();
        assert!(!engine.delivery_guard_is_current(&actor, &guard).await);
    }

    #[tokio::test]
    async fn durable_dispatcher_recovers_when_immediate_projection_has_no_live_channel() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('user','carmilla')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('server','user','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO roles(id,server_id,name,permissions,is_default) \
             VALUES('everyone','server','@everyone',?,1)",
        )
        .bind(crate::engine::permissions::DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let (_, actor) = auth.issue_web_session("user").await.unwrap();
        let engine = Arc::new(ChatEngine::new(
            pool.clone(),
            auth,
            "replay-secret",
            4000,
            100,
        ));
        let (session_id, mut receiver) = engine
            .connect(
                Some("user".into()),
                "carmilla".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine.bind_authenticated_actor(session_id, actor).unwrap();
        let shutdown = tokio_util::sync::CancellationToken::new();
        let receipt = engine
            .submit_channel_message(
                session_id,
                super::super::messaging::SendMessageCommand {
                    request_id: "request",
                    client_message_id: "client",
                    operation_generation: None,
                    conversation_id: None,
                    server_id: "server",
                    channel: "#general",
                    content: "durable",
                    content_format: super::super::messaging::ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                    mentions: &[],
                },
                None,
            )
            .await
            .unwrap();
        // The immediate path had no in-memory channel projection. Restore the
        // subscription before starting the worker to model restart recovery.
        let mut channel = ChannelState::new("channel".into(), "server".into(), "#general".into());
        channel.members.insert(session_id);
        engine.channels.insert("channel".into(), channel);
        let worker = tokio::spawn(engine.clone().run_delivery_dispatcher(shutdown.clone()));
        let delivered = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let event = receiver.recv().await.unwrap();
                let _guard = engine
                    .get_session(session_id)
                    .unwrap()
                    .take_delivery_guard();
                if let ChatEvent::DurableEvent { event } = event {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(delivered.entity_id, receipt.message_id);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let completed: bool = sqlx::query_scalar(
                    "SELECT completed_at IS NOT NULL FROM delivery_outbox WHERE event_sequence=?",
                )
                .bind(receipt.event_sequence_internal as i64)
                .fetch_one(&pool)
                .await
                .unwrap();
                if completed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shutdown.cancel();
        worker.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn retention_preserves_failed_gap_and_advances_replay_floor_contiguously() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('user','user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','user')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('channel','server','#general')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE channel_id='channel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        for index in 1..=3 {
            let sequence: i64 = sqlx::query_scalar(
                "INSERT INTO event_log(database_generation,conversation_id,event_kind,entity_type, \
                                       entity_id,entity_version,authorization_version,actor_id, \
                                       descriptor_json,created_at) \
                 VALUES(?,?,'test','metadata',?,1,0,'user','{}',datetime('now','-8 days')) \
                 RETURNING event_sequence",
            )
            .bind(&generation)
            .bind(&conversation)
            .bind(format!("entity-{index}"))
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO delivery_outbox(event_sequence,completed_at,last_error) \
                 VALUES(?,CASE WHEN ?=2 THEN NULL ELSE datetime('now') END, \
                          CASE WHEN ?=2 THEN 'injected failure' ELSE NULL END)",
            )
            .bind(sequence)
            .bind(index)
            .bind(index)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "UPDATE event_retention_state SET dispatcher_high_water=1,retention_seconds=3600 \
             WHERE singleton=1",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "secret".into(), 1);
        let engine = ChatEngine::new(pool.clone(), auth, "replay", 4000, 100);
        assert_eq!(engine.prune_delivery_retention().await.unwrap(), 1);
        let remaining: Vec<i64> =
            sqlx::query_scalar("SELECT event_sequence FROM event_log ORDER BY event_sequence")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec![2, 3]);
        let floor: i64 = sqlx::query_scalar(
            "SELECT retained_from_sequence FROM event_retention_state WHERE singleton=1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(floor, 2);
    }

    #[tokio::test]
    async fn test_join_and_message() {
        let engine = setup_engine();

        let (sid1, mut rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid2, mut rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid1, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        engine
            .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();

        while rx1.try_recv().is_ok() {}
        while rx2.try_recv().is_ok() {}

        engine
            .send_message(
                sid1,
                DEFAULT_SERVER_ID,
                "#general",
                "Hello from Alice!",
                None,
                None,
                None,
            )
            .unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message { from, content, .. } => {
                assert_eq!(from, "alice");
                assert_eq!(content, "Hello from Alice!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }

        // Sender receives a MessageAck (not the Message itself)
        let ack = rx1.try_recv().unwrap();
        assert!(matches!(ack, ChatEvent::MessageAck { .. }));
        assert!(rx1.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_part_channel() {
        let engine = setup_engine();

        let (sid1, mut rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid2, _rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid1, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        engine
            .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();

        while rx1.try_recv().is_ok() {}

        engine
            .part_channel(sid2, DEFAULT_SERVER_ID, "#general", None)
            .unwrap();

        let event = rx1.try_recv().unwrap();
        match event {
            ChatEvent::Part { nickname, .. } => assert_eq!(nickname, "bob"),
            _ => panic!("Expected Part event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_set_topic() {
        let engine = setup_engine();

        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        while rx.try_recv().is_ok() {}

        engine
            .set_topic(
                sid,
                DEFAULT_SERVER_ID,
                "#general",
                "Welcome to Concord!".into(),
            )
            .await
            .unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            ChatEvent::TopicChange { topic, .. } => {
                assert_eq!(topic, "Welcome to Concord!");
            }
            _ => panic!("Expected TopicChange event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_dm() {
        let engine = setup_engine();

        let (sid1, _rx1) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (_sid2, mut rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .send_message(sid1, DEFAULT_SERVER_ID, "bob", "Hey Bob!", None, None, None)
            .unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message {
                from,
                target,
                content,
                ..
            } => {
                assert_eq!(from, "alice");
                assert_eq!(target, "bob");
                assert_eq!(content, "Hey Bob!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn dm_fans_out_to_every_active_recipient_connection() {
        let engine = setup_engine();
        let (sender, _sender_rx) = engine
            .connect(
                Some("alice-id".into()),
                "alice".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        let (_, mut web_rx) = engine
            .connect(
                Some("bob-id".into()),
                "bob".into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        let (_, mut irc_rx) = engine
            .connect(Some("bob-id".into()), "bob".into(), Protocol::Irc, None)
            .unwrap();

        engine
            .send_message(
                sender,
                DEFAULT_SERVER_ID,
                "bob",
                "both devices",
                None,
                None,
                None,
            )
            .unwrap();

        for receiver in [&mut web_rx, &mut irc_rx] {
            assert!(matches!(
                receiver.try_recv(),
                Ok(ChatEvent::Message { ref content, .. }) if content == "both devices"
            ));
        }
    }

    #[tokio::test]
    async fn test_list_channels() {
        let engine = setup_engine();

        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#rust")
            .await
            .unwrap();

        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert_eq!(channels.len(), 2);

        let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"#general"));
        assert!(names.contains(&"#rust"));
    }

    #[tokio::test]
    async fn test_create_server() {
        let engine = setup_engine();

        let server_id = engine
            .create_server("Test Server".into(), "user1".into(), None)
            .await
            .unwrap();

        assert!(engine.servers.contains_key(&server_id));
        let channels = engine.list_channels(&server_id);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].name, "#general");
    }

    #[tokio::test]
    async fn test_server_isolation() {
        let engine = setup_engine();

        let server_a = engine
            .create_server("Server A".into(), "user1".into(), None)
            .await
            .unwrap();
        let server_b = engine
            .create_server("Server B".into(), "user1".into(), None)
            .await
            .unwrap();

        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid, &server_a, "#general")
            .await
            .unwrap();
        while rx.try_recv().is_ok() {}

        let (sid2, _rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid2, &server_b, "#general")
            .await
            .unwrap();

        // Alice is not in server_b's #general — should fail
        let result = engine.send_message(sid, &server_b, "#general", "Hello", None, None, None);
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Edge cases: sessions
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_disconnect_nonexistent_session_is_noop() {
        let engine = setup_engine();
        let fake_id = ConnectionId::new();
        // Should not panic
        engine.disconnect(fake_id);
    }

    #[tokio::test]
    async fn test_get_session_nonexistent() {
        let engine = setup_engine();
        assert!(engine.get_session(ConnectionId::new()).is_none());
    }

    #[tokio::test]
    async fn test_is_nick_available() {
        let engine = setup_engine();
        assert!(engine.is_nick_available("alice"));
        let (_sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        assert!(!engine.is_nick_available("alice"));
        assert!(engine.is_nick_available("bob"));
    }

    #[tokio::test]
    async fn test_connect_applies_protocol_specific_name_validation() {
        let engine = setup_engine();
        let result = engine.connect(None, "".into(), Protocol::WebSocket, None);
        assert!(result.is_err());

        let result = engine.connect(None, "has space!".into(), Protocol::WebSocket, None);
        assert!(result.is_ok());

        let result = engine.connect(None, "has space".into(), Protocol::Irc, None);
        assert!(result.is_err());

        let result = engine.connect(None, "1invalid".into(), Protocol::Irc, None);
        assert!(result.is_err());

        let result = engine.connect(None, "a".repeat(257), Protocol::WebSocket, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_connect_with_user_id_and_avatar() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(
                Some("user_123".into()),
                "alice".into(),
                Protocol::WebSocket,
                Some("https://example.com/avatar.png".into()),
            )
            .unwrap();
        let session = engine.get_session(sid).unwrap();
        assert_eq!(session.user_id.as_deref(), Some("user_123"));
        assert_eq!(
            session.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
    }

    #[tokio::test]
    async fn test_connect_irc_protocol() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "irc-user".into(), Protocol::Irc, None)
            .unwrap();
        let session = engine.get_session(sid).unwrap();
        assert_eq!(session.protocol, Protocol::Irc);
    }

    // ────────────────────────────────────────────────────────────────
    // Edge cases: channels and messaging
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_send_message_to_nonexistent_channel() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#nonexistent",
            "hello",
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_message_with_invalid_session() {
        let engine = setup_engine();
        let fake = ConnectionId::new();
        let result =
            engine.send_message(fake, DEFAULT_SERVER_ID, "#general", "hi", None, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_join_channel_nonexistent_server() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let result = engine.join_channel(sid, "no-such-server", "#general").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_join_channel_rejects_detached_creation() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let before = engine.list_channels(DEFAULT_SERVER_ID);
        assert!(
            engine
                .join_channel(sid, DEFAULT_SERVER_ID, "#new-channel")
                .await
                .is_err()
        );
        assert_eq!(engine.list_channels(DEFAULT_SERVER_ID).len(), before.len());
    }

    #[tokio::test]
    async fn test_join_channel_twice_is_ok() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        // Joining again should be a no-op, not an error
        let result = engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_part_channel_not_in() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        // Create the channel first by having someone join
        let (sid2, _rx2) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid2, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        // alice never joined, so parting should fail
        let result = engine.part_channel(sid, DEFAULT_SERVER_ID, "#general", None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_channel_name_normalization_on_join() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        // Joining with "General" should normalize to "#general"
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "General")
            .await
            .unwrap();
        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert!(channels.iter().any(|channel| channel.name == "#general"));
    }

    #[tokio::test]
    async fn test_message_not_echoed_to_sender() {
        let engine = setup_engine();
        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        // Drain join events
        while rx.try_recv().is_ok() {}

        engine
            .send_message(
                sid,
                DEFAULT_SERVER_ID,
                "#general",
                "hello",
                None,
                None,
                None,
            )
            .unwrap();
        // Message should NOT be echoed back to the sender (only a MessageAck)
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, ChatEvent::MessageAck { .. }));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_empty_message_rejected() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        let result = engine.send_message(sid, DEFAULT_SERVER_ID, "#general", "", None, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_whitespace_only_message_rejected() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        let result =
            engine.send_message(sid, DEFAULT_SERVER_ID, "#general", "   ", None, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oversized_message_rejected() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        let long_msg = "x".repeat(4001);
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            &long_msg,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_message_at_max_length_accepted() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        let max_msg = "x".repeat(4000);
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            &max_msg,
            None,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    // ────────────────────────────────────────────────────────────────
    // Topic edge cases
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_set_topic_too_long() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        let long_topic = "t".repeat(501);
        let result = engine
            .set_topic(sid, DEFAULT_SERVER_ID, "#general", long_topic)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_topic_empty_clears() {
        let engine = setup_engine();
        let (sid, mut rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        while rx.try_recv().is_ok() {}

        // Set a topic first
        engine
            .set_topic(sid, DEFAULT_SERVER_ID, "#general", "Hello".into())
            .await
            .unwrap();
        while rx.try_recv().is_ok() {}

        // Clear topic
        engine
            .set_topic(sid, DEFAULT_SERVER_ID, "#general", "".into())
            .await
            .unwrap();
        let event = rx.try_recv().unwrap();
        match event {
            ChatEvent::TopicChange { topic, .. } => {
                assert_eq!(topic, "");
            }
            _ => panic!("Expected TopicChange event"),
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Server management
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_server_invalid_name() {
        let engine = setup_engine();
        // Empty name
        let result = engine.create_server("".into(), "user1".into(), None).await;
        assert!(result.is_err());

        // Whitespace-only name
        let result = engine
            .create_server("   ".into(), "user1".into(), None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_server_too_long_name() {
        let engine = setup_engine();
        let long_name = "a".repeat(101);
        let result = engine.create_server(long_name, "user1".into(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_server_max_name_length() {
        let engine = setup_engine();
        let max_name = "a".repeat(100);
        let result = engine.create_server(max_name, "user1".into(), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_server_with_icon() {
        let engine = setup_engine();
        let server_id = engine
            .create_server(
                "My Server".into(),
                "user1".into(),
                Some("https://example.com/icon.png".into()),
            )
            .await
            .unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert_eq!(
            server.icon_url.as_deref(),
            Some("https://example.com/icon.png")
        );
    }

    #[tokio::test]
    async fn test_find_server_by_name() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test Server".into(), "user1".into(), None)
            .await
            .unwrap();
        // Case insensitive lookup
        assert_eq!(
            engine.find_server_by_name("test server"),
            Some(server_id.clone())
        );
        assert_eq!(engine.find_server_by_name("TEST SERVER"), Some(server_id));
        assert!(engine.find_server_by_name("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_get_server_name() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("My Server".into(), "user1".into(), None)
            .await
            .unwrap();
        assert_eq!(
            engine.get_server_name(&server_id),
            Some("My Server".to_string())
        );
        assert!(engine.get_server_name("nonexistent").is_none());
    }

    // ────────────────────────────────────────────────────────────────
    // Multiple channels and users
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_multiple_channels_message_isolation() {
        let engine = setup_engine();
        let (sid_alice, mut rx_alice) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid_bob, mut rx_bob) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        // Alice joins #general, Bob joins #rust
        engine
            .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        engine
            .join_channel(sid_bob, DEFAULT_SERVER_ID, "#rust")
            .await
            .unwrap();
        while rx_alice.try_recv().is_ok() {}
        while rx_bob.try_recv().is_ok() {}

        // Alice sends to #general — Bob should NOT receive it (different channel)
        engine
            .send_message(
                sid_alice,
                DEFAULT_SERVER_ID,
                "#general",
                "hello general",
                None,
                None,
                None,
            )
            .unwrap();
        assert!(rx_bob.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_disconnect_broadcasts_quit() {
        let engine = setup_engine();
        let (sid_alice, _rx_alice) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid_bob, mut rx_bob) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        engine
            .join_channel(sid_bob, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        while rx_bob.try_recv().is_ok() {}

        engine.disconnect(sid_alice);

        let event = rx_bob.try_recv().unwrap();
        match event {
            ChatEvent::Quit { nickname, .. } => assert_eq!(nickname, "alice"),
            _ => panic!("Expected Quit event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_disconnect_removes_from_channel() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();

        engine.disconnect(sid);

        // Channel should have 0 members now
        let channels = engine.list_channels(DEFAULT_SERVER_ID);
        assert_eq!(channels[0].member_count, 0);
    }

    // ────────────────────────────────────────────────────────────────
    // Normalize channel name additional tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_normalize_channel_name_already_lowercase() {
        assert_eq!(normalize_channel_name("#already-lower"), "#already-lower");
    }

    #[test]
    fn test_normalize_channel_name_mixed_case() {
        assert_eq!(normalize_channel_name("MixedCase"), "#mixedcase");
    }

    #[test]
    fn test_normalize_channel_name_uppercase_with_hash() {
        assert_eq!(normalize_channel_name("#UPPER"), "#upper");
    }

    #[test]
    fn test_normalize_channel_name_with_numbers() {
        assert_eq!(normalize_channel_name("channel123"), "#channel123");
    }

    // ────────────────────────────────────────────────────────────────
    // DM edge cases
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dm_to_nonexistent_user() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let result =
            engine.send_message(sid, DEFAULT_SERVER_ID, "nobody", "hello", None, None, None);
        // DMs to non-existent users fail because there's no channel and no user session
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Rate limiting
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_message_rate_limiting() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();

        // The default rate limiter allows burst of 10.
        // Send 10 messages — all should succeed.
        for i in 0..10 {
            let result = engine.send_message(
                sid,
                DEFAULT_SERVER_ID,
                "#general",
                &format!("msg {i}"),
                None,
                None,
                None,
            );
            assert!(result.is_ok(), "Message {i} should succeed");
        }

        // 11th should be rate-limited
        let result = engine.send_message(
            sid,
            DEFAULT_SERVER_ID,
            "#general",
            "msg 10",
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────
    // Server owner and join_server (in-memory only, no DB)
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_server_sets_owner() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test".into(), "user1".into(), None)
            .await
            .unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert_eq!(server.owner_id, "user1");
    }

    #[tokio::test]
    async fn test_join_server_nonexistent() {
        let engine = setup_engine();
        let result = engine.join_server("user1", "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_join_server_adds_member() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test".into(), "owner".into(), None)
            .await
            .unwrap();
        engine.join_server("user1", &server_id).await.unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert!(server.member_user_ids.contains("user1"));
    }

    #[tokio::test]
    async fn test_leave_server_removes_member() {
        let engine = setup_engine();
        let server_id = engine
            .create_server("Test".into(), "owner".into(), None)
            .await
            .unwrap();
        engine.join_server("user1", &server_id).await.unwrap();
        engine.leave_server("user1", &server_id).await.unwrap();
        let server = engine.servers.get(&server_id).unwrap();
        assert!(!server.member_user_ids.contains("user1"));
    }

    // ────────────────────────────────────────────────────────────────
    // Part channel with reason
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_part_channel_with_reason() {
        let engine = setup_engine();
        let (sid_alice, mut rx_alice) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        let (sid_bob, _rx_bob) = engine
            .connect(None, "bob".into(), Protocol::WebSocket, None)
            .unwrap();

        engine
            .join_channel(sid_alice, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        engine
            .join_channel(sid_bob, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        while rx_alice.try_recv().is_ok() {}

        engine
            .part_channel(
                sid_bob,
                DEFAULT_SERVER_ID,
                "#general",
                Some("bye!".to_string()),
            )
            .unwrap();

        let event = rx_alice.try_recv().unwrap();
        match event {
            ChatEvent::Part {
                nickname, reason, ..
            } => {
                assert_eq!(nickname, "bob");
                assert_eq!(reason, Some("bye!".to_string()));
            }
            _ => panic!("Expected Part event"),
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Resolve channel ID
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_resolve_channel_id_nonexistent() {
        let engine = setup_engine();
        let result = engine.resolve_channel_id(DEFAULT_SERVER_ID, "#nothing");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_channel_id_after_join() {
        let engine = setup_engine();
        let (sid, _rx) = engine
            .connect(None, "alice".into(), Protocol::WebSocket, None)
            .unwrap();
        engine
            .join_channel(sid, DEFAULT_SERVER_ID, "#general")
            .await
            .unwrap();
        let result = engine.resolve_channel_id(DEFAULT_SERVER_ID, "#general");
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    // ────────────────────────────────────────────────────────────────
    // list_servers_for_user (in-memory, no DB)
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_list_servers_for_user() {
        let engine = setup_engine();
        let sid1 = engine
            .create_server("Server A".into(), "user1".into(), None)
            .await
            .unwrap();
        let sid2 = engine
            .create_server("Server B".into(), "user1".into(), None)
            .await
            .unwrap();
        let _ = engine
            .create_server("Server C".into(), "user2".into(), None)
            .await
            .unwrap();

        // user1 should see Server A and Server B (they're the owner)
        engine.join_server("user1", &sid1).await.unwrap();
        engine.join_server("user1", &sid2).await.unwrap();
        let servers = engine.list_servers_for_user("user1").await;
        assert_eq!(servers.len(), 2);
        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Server A"));
        assert!(names.contains(&"Server B"));
    }

    // ────────────────────────────────────────────────────────────────
    // Multiple server creation generates unique IDs
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_server_ids_are_unique() {
        let engine = setup_engine();
        let id1 = engine
            .create_server("S1".into(), "user1".into(), None)
            .await
            .unwrap();
        let id2 = engine
            .create_server("S2".into(), "user1".into(), None)
            .await
            .unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn typed_search_parser_preserves_unicode_phrases_and_filters() {
        let parsed = parse_search_query(
            "\"café au lait\" from:Laurelai in:#general has:attachment has:link before:2026-09-05 after:2026-09-01",
        )
        .unwrap();
        assert_eq!(parsed.text.as_deref(), Some("café au lait"));
        assert_eq!(parsed.sender.as_deref(), Some("Laurelai"));
        assert_eq!(parsed.channel.as_deref(), Some("#general"));
        assert!(parsed.has_attachment && parsed.has_link);
        assert_eq!(parsed.before.as_deref(), Some("2026-09-05T00:00:00+00:00"));
        assert_eq!(parsed.after.as_deref(), Some("2026-09-02T00:00:00+00:00"));
    }

    #[test]
    fn typed_search_parser_accepts_each_filter_without_text() {
        let cases = [
            ("from:Alice", "sender"),
            ("in:#general", "channel"),
            ("has:attachment", "attachment"),
            ("has:link", "link"),
            ("before:2026-09-05", "before"),
            ("after:2026-09-01", "after"),
        ];
        for (query, expected) in cases {
            let parsed = parse_search_query(query)
                .unwrap_or_else(|error| panic!("rejected {expected} filter {query:?}: {error}"));
            assert!(parsed.text.is_none(), "filter parsed as text: {query:?}");
        }
    }

    #[test]
    fn typed_search_parser_rejects_invalid_filters_and_unicode_control_input() {
        for query in [
            "has:executable",
            "before:yesterday",
            "from:",
            "from:alice from:bob",
            "in:#one in:#two",
            "before:2026-09-01 before:2026-09-02",
            "after:2026-09-01 after:2026-09-02",
            "\"unfinished",
            "hello\nworld",
            "",
        ] {
            assert!(parse_search_query(query).is_err(), "accepted {query:?}");
        }
        assert!(parse_search_query(&"x".repeat(1_025)).is_err());
    }
}
