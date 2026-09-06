use sqlx::{Row, SqliteConnection, SqlitePool};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService};
use crate::db::models::{
    ChannelFollowRow, EventRsvpRow, InviteRow, ServerEventRow, ServerRow, ServerTemplateRow,
};

use super::authorization::{AuthorizationService, AuthorizationStamp, ChannelAction};
use super::ids::{ChannelId, ResourceIdError, ServerId};
use super::permissions::Permissions;

#[derive(Debug)]
pub enum CommunityError {
    Authentication(crate::auth::authority::AuthError),
    Forbidden,
    InvalidInput(&'static str),
    Conflict(&'static str),
    Admission(super::write_admission::WriteAdmissionError),
    Database(sqlx::Error),
    InvalidStoredResourceId(ResourceIdError),
}

pub struct PublicInvitePreview {
    pub code: String,
    pub server_id: String,
    pub server_name: String,
    pub server_icon_url: Option<String>,
    pub is_vanity: bool,
}

pub enum PublicInvitePreviewError {
    ExpiredOrExhausted,
    DependencyUnavailable,
    Database(sqlx::Error),
}

#[derive(sqlx::FromRow)]
struct PublicInviteRow {
    code: String,
    server_id: String,
    expires_at: Option<String>,
    max_uses: Option<i32>,
    use_count: i32,
    server_name: String,
    server_icon_url: Option<String>,
}

impl CommunityError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            )
            | Self::Admission(_)
            | Self::Database(_)
            | Self::InvalidStoredResourceId(_) => "DEPENDENCY_UNAVAILABLE",
            Self::Authentication(_) => "UNAUTHENTICATED",
            Self::Forbidden => "FORBIDDEN",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Conflict(_) => "CONFLICT",
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            )
            | Self::Admission(_)
            | Self::Database(_)
            | Self::InvalidStoredResourceId(_) => "dependency unavailable",
            Self::Authentication(_) => "authentication required",
            Self::Forbidden => "resource unavailable",
            Self::InvalidInput(message) => message,
            Self::Conflict(message) => message,
        }
    }
}

impl std::fmt::Display for CommunityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.safe_message())
    }
}

impl std::error::Error for CommunityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Authentication(error) => Some(error),
            Self::Admission(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::InvalidStoredResourceId(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for CommunityError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}
impl From<ResourceIdError> for CommunityError {
    fn from(error: ResourceIdError) -> Self {
        Self::InvalidStoredResourceId(error)
    }
}
impl From<super::write_admission::WriteAdmissionError> for CommunityError {
    fn from(error: super::write_admission::WriteAdmissionError) -> Self {
        Self::Admission(error)
    }
}
impl From<crate::auth::authority::AuthError> for CommunityError {
    fn from(error: crate::auth::authority::AuthError) -> Self {
        Self::Authentication(error)
    }
}
impl From<super::authorization::AuthorizationError> for CommunityError {
    fn from(error: super::authorization::AuthorizationError) -> Self {
        match error {
            super::authorization::AuthorizationError::Unavailable => Self::Forbidden,
            super::authorization::AuthorizationError::Database(error) => Self::Database(error),
            super::authorization::AuthorizationError::Authentication(error) => {
                Self::Authentication(error)
            }
        }
    }
}
impl From<CommunityError> for String {
    fn from(error: CommunityError) -> Self {
        format!("{}: {}", error.code(), error.safe_message())
    }
}

pub struct RedeemedInvite {
    pub server_id: ServerId,
}

pub struct CreatedInvite {
    pub id: String,
    pub code: String,
    pub created_at: String,
}

pub struct CreatedFollow {
    pub id: String,
    pub created_by: String,
}

pub struct UpdateCommunityParams<'a> {
    pub server_id: &'a ServerId,
    pub description: Option<&'a str>,
    pub discoverable: bool,
    pub welcome: Option<&'a str>,
    pub rules: Option<&'a str>,
    pub category: Option<&'a str>,
}

pub struct CreateEvent<'a> {
    pub id: &'a str,
    pub server_id: &'a ServerId,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub channel_id: Option<&'a ChannelId>,
    pub start_time: &'a str,
    pub end_time: Option<&'a str>,
    pub image_url: Option<&'a str>,
    pub created_by: &'a str,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateConfig {
    format_version: i64,
    channels: Vec<TemplateChannel>,
    categories: Vec<TemplateCategory>,
    roles: Vec<TemplateRole>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateChannel {
    id: String,
    name: String,
    topic: String,
    category_id: Option<String>,
    position: i32,
    is_private: bool,
    channel_type: String,
    slowmode_seconds: i32,
    is_nsfw: bool,
    is_announcement: bool,
    is_default: bool,
    aliases: Vec<String>,
    role_overrides: Vec<TemplateRoleOverride>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateCategory {
    id: String,
    name: String,
    position: i32,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateRole {
    id: String,
    name: String,
    color: Option<String>,
    position: i32,
    permissions: i64,
    is_default: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateRoleOverride {
    role_id: String,
    allow_bits: i64,
    deny_bits: i64,
}

pub struct CreatedTemplate {
    pub id: String,
    pub created_at: String,
}

pub struct VisibleEvent {
    pub event: ServerEventRow,
    pub rsvp_count: i64,
}

impl TemplateConfig {
    fn validate(&self) -> Result<(), CommunityError> {
        if self.format_version != 1
            || self.channels.len() > 500
            || self.categories.len() > 100
            || self.roles.len() > 250
        {
            return Err(CommunityError::InvalidInput("invalid server template"));
        }
        let category_ids: HashSet<&str> = self
            .categories
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        let channel_ids: HashSet<&str> =
            self.channels.iter().map(|item| item.id.as_str()).collect();
        let role_ids: HashSet<&str> = self.roles.iter().map(|item| item.id.as_str()).collect();
        if category_ids.len() != self.categories.len()
            || channel_ids.len() != self.channels.len()
            || role_ids.len() != self.roles.len()
            || self.roles.iter().filter(|role| role.is_default).count() != 1
            || self
                .channels
                .iter()
                .filter(|channel| channel.is_default)
                .count()
                != 1
        {
            return Err(CommunityError::InvalidInput("invalid server template"));
        }
        for category in &self.categories {
            crate::engine::validation::validate_server_name(&category.name)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
            if category.position < 0 {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
        }
        let mut aliases = HashSet::new();
        for channel in &self.channels {
            crate::engine::validation::validate_channel_name(&channel.name)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
            crate::engine::validation::validate_topic(&channel.topic)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
            if channel.position < 0
                || !(0..=21_600).contains(&channel.slowmode_seconds)
                || !matches!(channel.channel_type.as_str(), "text" | "forum")
                || channel
                    .category_id
                    .as_deref()
                    .is_some_and(|id| !category_ids.contains(id))
            {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
            let mut override_roles = HashSet::new();
            if channel.aliases.iter().any(|alias| {
                alias.is_empty()
                    || alias.len() > 100
                    || !alias.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                    })
                    || !aliases.insert(alias.to_ascii_lowercase())
            }) || channel.role_overrides.iter().any(|rule| {
                !role_ids.contains(rule.role_id.as_str())
                    || !override_roles.insert(rule.role_id.as_str())
                    || rule.allow_bits < 0
                    || rule.deny_bits < 0
                    || Permissions::from_bits(rule.allow_bits as u64).is_none()
                    || Permissions::from_bits(rule.deny_bits as u64).is_none()
            }) {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
            if channel.is_default && channel.is_private {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
        }
        for role in &self.roles {
            if role.id.is_empty()
                || role.name.trim().is_empty()
                || role.name.len() > 100
                || role.position < 0
                || role.permissions < 0
                || Permissions::from_bits(role.permissions as u64).is_none()
            {
                return Err(CommunityError::InvalidInput("invalid server template"));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct CommunityService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

impl CommunityService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self {
            pool: pool.clone(),
            authorization: AuthorizationService::new(pool.clone()),
            auth,
            writes,
        }
    }

    pub async fn create_invite(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        max_uses: Option<i32>,
        expires_at: Option<&str>,
        channel_id: Option<&ChannelId>,
    ) -> Result<CreatedInvite, CommunityError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.map(ChannelId::as_str);
        if max_uses.is_some_and(|value| value <= 0) {
            return Err(CommunityError::InvalidInput("invalid invite use limit"));
        }
        if expires_at.is_some_and(|value| value.len() > 64 || value.chars().any(char::is_control)) {
            return Err(CommunityError::InvalidInput("invalid invite expiry"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::CREATE_INVITES,
            )
            .await?;
        if let Some(expires_at) = expires_at {
            let future: bool =
                sqlx::query_scalar("SELECT unixepoch(?) IS NOT NULL AND unixepoch(?)>unixepoch()")
                    .bind(expires_at)
                    .bind(expires_at)
                    .fetch_one(&mut *tx)
                    .await?;
            if !future {
                return Err(CommunityError::InvalidInput("invalid invite expiry"));
            }
        }
        if let Some(channel_id) = channel_id {
            let scoped:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=? AND parent_channel_id IS NULL)")
                .bind(channel_id).bind(server_id).fetch_one(&mut *tx).await?;
            if !scoped {
                return Err(CommunityError::Forbidden);
            }
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await?;
        }
        let id = Uuid::new_v4().to_string();
        use rand::RngExt;
        let code: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(24)
            .map(char::from)
            .collect();
        let created_at:String=sqlx::query_scalar("INSERT INTO invites(id,server_id,code,created_by,max_uses,expires_at,channel_id) VALUES(?,?,?,?,?,?,?) RETURNING created_at")
            .bind(&id).bind(server_id).bind(&code).bind(actor.user_id().as_str()).bind(max_uses).bind(expires_at).bind(channel_id)
            .fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(CreatedInvite {
            id,
            code,
            created_at,
        })
    }

    pub async fn delete_invite(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        invite_id: &str,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let deleted = sqlx::query("DELETE FROM invites WHERE id=? AND server_id=?")
            .bind(invite_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_invites(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(Vec<InviteRow>, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let rows = sqlx::query_as::<_, InviteRow>(
            "SELECT * FROM invites WHERE server_id=? ORDER BY created_at DESC",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((rows, stamp))
    }

    pub async fn update_community(
        &self,
        actor: &Actor,
        params: &UpdateCommunityParams<'_>,
    ) -> Result<bool, CommunityError> {
        let server_id = params.server_id.as_str();
        if params.description.is_some_and(|value| value.len() > 2_000)
            || params.welcome.is_some_and(|value| value.len() > 2_000)
            || params.rules.is_some_and(|value| value.len() > 20_000)
            || params
                .category
                .is_some_and(|value| value.len() > 100 || value.chars().any(char::is_control))
        {
            return Err(CommunityError::InvalidInput("invalid community settings"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        sqlx::query("UPDATE servers SET description=?,is_discoverable=?,welcome_message=?,rules_text=?,category=?,rules_version=rules_version+CASE WHEN rules_text IS NOT ? THEN 1 ELSE 0 END,updated_at=datetime('now') WHERE id=?")
            .bind(params.description).bind(i64::from(params.discoverable)).bind(params.welcome).bind(params.rules).bind(params.category).bind(params.rules).bind(server_id)
            .execute(&mut *tx).await?;
        let rules_accepted: bool = sqlx::query_scalar(
            "SELECT accepted_rules_version=(SELECT rules_version FROM servers WHERE id=?) \
             FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        tx.commit().await?;
        Ok(rules_accepted)
    }

    pub async fn get_community(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(ServerRow, bool, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await?;
        let server = sqlx::query_as::<_, ServerRow>("SELECT * FROM servers WHERE id=?")
            .bind(server_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CommunityError::Forbidden)?;
        let rules_accepted: bool = sqlx::query_scalar(
            "SELECT accepted_rules_version=(SELECT rules_version FROM servers WHERE id=?) \
             FROM server_members WHERE server_id=? AND user_id=?",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((server, rules_accepted, stamp))
    }

    pub async fn discover(
        &self,
        actor: &Actor,
        category: Option<&str>,
    ) -> Result<Vec<ServerRow>, CommunityError> {
        if category.is_some_and(|value| {
            value.is_empty() || value.len() > 100 || value.chars().any(char::is_control)
        }) {
            return Err(CommunityError::InvalidInput("invalid community category"));
        }
        let mut tx = self.pool.begin().await?;
        self.auth.validate_actor_in(&mut tx, actor).await?;
        let rows = match category {
            Some(category) => {
                sqlx::query_as::<_, ServerRow>(
                    "SELECT * FROM servers WHERE is_discoverable=1 AND category=? \
                     ORDER BY name,id LIMIT 100",
                )
                .bind(category)
                .fetch_all(&mut *tx)
                .await?
            }
            None => {
                sqlx::query_as::<_, ServerRow>(
                    "SELECT * FROM servers WHERE is_discoverable=1 ORDER BY name,id LIMIT 100",
                )
                .fetch_all(&mut *tx)
                .await?
            }
        };
        tx.commit().await?;
        Ok(rows)
    }

    pub async fn discover_public(
        &self,
        category: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ServerRow>, CommunityError> {
        if category.is_some_and(|value| {
            value.is_empty() || value.len() > 100 || value.chars().any(char::is_control)
        }) {
            return Err(CommunityError::InvalidInput("invalid community category"));
        }
        let rows = sqlx::query_as::<_, ServerRow>(
            "SELECT * FROM servers WHERE is_discoverable=1 \
             AND (? IS NULL OR category=?) ORDER BY name,id LIMIT ? OFFSET ?",
        )
        .bind(category)
        .bind(category)
        .bind(limit.clamp(1, 100))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn public_invite_preview(
        &self,
        code: &str,
    ) -> Result<Option<PublicInvitePreview>, PublicInvitePreviewError> {
        let invitation: Option<PublicInviteRow> = sqlx::query_as(
            "SELECT i.code,s.id AS server_id,i.expires_at,i.max_uses,i.use_count, \
             s.name AS server_name,s.icon_url AS server_icon_url \
             FROM invites i JOIN servers s ON s.id=i.server_id WHERE i.code=?",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(PublicInvitePreviewError::Database)?;
        if let Some(invitation) = invitation {
            let expired = invitation.expires_at.is_some_and(|expires_at| {
                expires_at < chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
            });
            if expired
                || invitation
                    .max_uses
                    .is_some_and(|maximum| invitation.use_count >= maximum)
            {
                return Err(PublicInvitePreviewError::ExpiredOrExhausted);
            }
            return Ok(Some(PublicInvitePreview {
                code: invitation.code,
                server_id: invitation.server_id,
                server_name: invitation.server_name,
                server_icon_url: invitation.server_icon_url,
                is_vanity: false,
            }));
        }
        let vanity: Option<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id,name,icon_url FROM servers WHERE vanity_code=?")
                .bind(code)
                .fetch_optional(&self.pool)
                .await
                .map_err(PublicInvitePreviewError::Database)?;
        Ok(vanity.map(
            |(server_id, server_name, server_icon_url)| PublicInvitePreview {
                code: code.to_owned(),
                server_id,
                server_name,
                server_icon_url,
                is_vanity: true,
            },
        ))
    }

    pub async fn set_vanity_code(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        vanity_code: Option<&str>,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        if let Some(vanity_code) = vanity_code {
            crate::engine::validation::validate_vanity_code(vanity_code)
                .map_err(|_| CommunityError::InvalidInput("invalid vanity code"))?;
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        if let Some(vanity_code) = vanity_code {
            let unavailable: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM servers WHERE vanity_code=? AND id<>?)",
            )
            .bind(vanity_code)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await?;
            if unavailable {
                return Err(CommunityError::Conflict("vanity code unavailable"));
            }
        }
        let updated =
            sqlx::query("UPDATE servers SET vanity_code=?,updated_at=datetime('now') WHERE id=?")
                .bind(vanity_code)
                .bind(server_id)
                .execute(&mut *tx)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        let audit_id = Uuid::new_v4().to_string();
        let changes = serde_json::json!({"vanity_code": vanity_code}).to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut tx,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "server_vanity_update",
                target_type: Some("server"),
                target_id: Some(server_id),
                reason: None,
                changes: Some(&changes),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_event(
        &self,
        actor: &Actor,
        params: &CreateEvent<'_>,
    ) -> Result<String, CommunityError> {
        let server_id = params.server_id.as_str();
        let channel_id = params.channel_id.map(ChannelId::as_str);
        if params.name.trim().is_empty()
            || params.name.chars().count() > 100
            || params.name.chars().any(char::is_control)
            || params.description.is_some_and(|value| value.len() > 2_000)
            || params.image_url.is_some_and(|value| value.len() > 2_000)
            || params.created_by != actor.user_id().as_str()
        {
            return Err(CommunityError::InvalidInput("invalid event"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let valid_times: bool = sqlx::query_scalar(
            "SELECT unixepoch(?) IS NOT NULL AND (? IS NULL OR (unixepoch(?) IS NOT NULL AND unixepoch(?)>unixepoch(?)))",
        ).bind(params.start_time).bind(params.end_time).bind(params.end_time)
            .bind(params.end_time).bind(params.start_time).fetch_one(&mut *tx).await?;
        if !valid_times {
            return Err(CommunityError::InvalidInput("invalid event time"));
        }
        if let Some(channel_id) = channel_id {
            let scoped: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM channels WHERE id=? AND server_id=?)",
            )
            .bind(channel_id)
            .bind(server_id)
            .fetch_one(&mut *tx)
            .await?;
            if !scoped {
                return Err(CommunityError::Forbidden);
            }
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await?;
        }
        let created_at: String = sqlx::query_scalar("INSERT INTO server_events(id,server_id,name,description,channel_id,start_time,end_time,image_url,created_by) VALUES(?,?,?,?,?,?,?,?,?) RETURNING created_at")
            .bind(params.id).bind(server_id).bind(params.name.trim()).bind(params.description)
            .bind(channel_id).bind(params.start_time).bind(params.end_time).bind(params.image_url)
            .bind(actor.user_id().as_str()).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(created_at)
    }

    pub async fn list_events(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(Vec<VisibleEvent>, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::VIEW_CHANNELS,
            )
            .await?;
        let rows = sqlx::query_as::<_, ServerEventRow>(
            "SELECT * FROM server_events WHERE server_id=? AND integrity_state='active' \
             ORDER BY start_time,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut events = Vec::with_capacity(rows.len());
        let mut channel_ids = Vec::new();
        for event in rows {
            if let Some(channel_id) = event.channel_id.as_deref() {
                match self
                    .authorization
                    .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                    .await
                {
                    Ok(()) => channel_ids.push(channel_id.to_owned()),
                    Err(super::authorization::AuthorizationError::Unavailable) => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            let rsvp_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM event_rsvps WHERE event_id=? AND status IN ('interested','going')",
            )
            .bind(&event.id)
            .fetch_one(&mut *tx)
            .await?;
            events.push(VisibleEvent { event, rsvp_count });
        }
        channel_ids.sort();
        channel_ids.dedup();
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &channel_ids)
            .await?;
        tx.commit().await?;
        Ok((events, stamp))
    }

    pub async fn set_announcement(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        value: bool,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let updated = sqlx::query("UPDATE channels SET is_announcement=? WHERE id=? AND server_id=? AND channel_type='text' AND parent_channel_id IS NULL")
            .bind(i64::from(value)).bind(channel_id).bind(server_id).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn follow_channel(
        &self,
        actor: &Actor,
        source: &ChannelId,
        target: &ChannelId,
    ) -> Result<CreatedFollow, CommunityError> {
        let source = source.as_str();
        let target = target.as_str();
        if source == target {
            return Err(CommunityError::InvalidInput(
                "announcement follows cannot form a cycle",
            ));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT id,server_id,is_announcement FROM channels WHERE id IN (?,?)")
                .bind(source)
                .bind(target)
                .fetch_all(&mut *tx)
                .await?;
        let _source_server = rows
            .iter()
            .find(|row| row.0 == source && row.2 != 0)
            .map(|row| row.1.as_str())
            .ok_or(CommunityError::Forbidden)?;
        let target_server = rows
            .iter()
            .find(|row| row.0 == target)
            .map(|row| row.1.as_str())
            .ok_or(CommunityError::Forbidden)?;
        self.authorization
            .authorize_actor_in(
                &mut tx,
                &self.auth,
                actor,
                source,
                ChannelAction::ManageMessages,
            )
            .await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                target_server,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        self.authorization
            .authorize_actor_in(&mut tx, &self.auth, actor, target, ChannelAction::Manage)
            .await?;
        let cycle: bool = sqlx::query_scalar(
            "WITH RECURSIVE reachable(id) AS (SELECT target_channel_id FROM channel_follows WHERE source_channel_id=? UNION SELECT f.target_channel_id FROM channel_follows f JOIN reachable r ON f.source_channel_id=r.id) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?)",
        ).bind(target).bind(source).fetch_one(&mut *tx).await?;
        if cycle {
            return Err(CommunityError::InvalidInput(
                "announcement follows cannot form a cycle",
            ));
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO channel_follows(id,source_channel_id,target_channel_id,created_by) VALUES(?,?,?,?)")
            .bind(&id).bind(source).bind(target).bind(actor.user_id().as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(CreatedFollow {
            id,
            created_by: actor.user_id().as_str().to_owned(),
        })
    }

    pub async fn unfollow_channel(
        &self,
        actor: &Actor,
        follow_id: &str,
    ) -> Result<ServerId, CommunityError> {
        let (_permit, mut tx) = self.writes.begin().await?;
        let target_server: Option<String> = sqlx::query_scalar(
            "SELECT c.server_id FROM channel_follows f JOIN channels c ON c.id=f.target_channel_id WHERE f.id=?",
        ).bind(follow_id).fetch_optional(&mut *tx).await?;
        let target_server = target_server.ok_or(CommunityError::Forbidden)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                &target_server,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        sqlx::query("DELETE FROM channel_follows WHERE id=?")
            .bind(follow_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(ServerId::from_stored(target_server)?)
    }

    pub async fn list_channel_follows(
        &self,
        actor: &Actor,
        channel_id: &ChannelId,
    ) -> Result<(Vec<ChannelFollowRow>, AuthorizationStamp), CommunityError> {
        let channel_id = channel_id.as_str();
        let mut tx = self.pool.begin().await?;
        let server_id: String = sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
            .bind(channel_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CommunityError::Forbidden)?;
        self.authorization
            .require_channel_actor_permission_in(
                &mut tx,
                &self.auth,
                actor,
                &server_id,
                channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let rows = sqlx::query_as::<_, ChannelFollowRow>(
            "SELECT * FROM channel_follows WHERE source_channel_id=? OR target_channel_id=? \
             ORDER BY created_at,id",
        )
        .bind(channel_id)
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, &server_id, &[channel_id.to_owned()])
            .await?;
        tx.commit().await?;
        Ok((rows, stamp))
    }

    async fn require_private_template_authority_in(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(), CommunityError> {
        let privileged: i64 = sqlx::query_scalar(
            "SELECT EXISTS( \
                SELECT 1 FROM servers s \
                JOIN server_members sm ON sm.server_id=s.id AND sm.user_id=? \
                WHERE s.id=? AND (s.owner_id=? OR sm.role IN ('owner','admin') OR EXISTS( \
                    SELECT 1 FROM user_roles ur JOIN roles r ON r.id=ur.role_id \
                    WHERE ur.server_id=s.id AND ur.user_id=sm.user_id \
                      AND (r.permissions & ?) != 0 \
                )) \
            )",
        )
        .bind(actor.user_id().as_str())
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .bind(Permissions::ADMINISTRATOR.bits() as i64)
        .fetch_one(connection)
        .await?;
        if privileged == 0 {
            return Err(CommunityError::Forbidden);
        }
        Ok(())
    }

    pub async fn create_template(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        name: &str,
        description: Option<&str>,
    ) -> Result<CreatedTemplate, CommunityError> {
        let server_id = server_id.as_str();
        crate::engine::validation::validate_server_name(name)
            .map_err(|_| CommunityError::InvalidInput("invalid template name"))?;
        if description
            .is_some_and(|value| value.len() > 1_000 || value.chars().any(char::is_control))
        {
            return Err(CommunityError::InvalidInput("invalid template description"));
        }
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;

        let has_private_channels: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE server_id=? \
             AND parent_channel_id IS NULL AND channel_type IN ('text','forum') \
             AND is_private=1)",
        )
        .bind(server_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_private_channels != 0 {
            self.require_private_template_authority_in(&mut tx, actor, server_id)
                .await?;
        }

        let category_rows = sqlx::query(
            "SELECT id,name,position FROM channel_categories WHERE server_id=? ORDER BY position,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let categories = category_rows
            .into_iter()
            .map(|row| TemplateCategory {
                id: row.get(0),
                name: row.get(1),
                position: row.get(2),
            })
            .collect();
        let role_rows = sqlx::query(
            "SELECT id,name,color,position,permissions,is_default FROM roles \
             WHERE server_id=? ORDER BY position,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let roles = role_rows
            .into_iter()
            .map(|row| TemplateRole {
                id: row.get(0),
                name: row.get(1),
                color: row.get(2),
                position: row.get(3),
                permissions: row.get(4),
                is_default: row.get::<i64, _>(5) != 0,
            })
            .collect();
        let channel_rows = sqlx::query(
            "SELECT id,name,topic,category_id,position,is_private,channel_type, \
                    slowmode_seconds,is_nsfw,is_announcement,is_default \
             FROM channels WHERE server_id=? AND parent_channel_id IS NULL \
               AND channel_type IN ('text','forum') \
             ORDER BY position,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut channels = Vec::with_capacity(channel_rows.len());
        for row in channel_rows {
            let channel_id: String = row.get(0);
            let aliases = sqlx::query_scalar(
                "SELECT alias FROM channel_aliases WHERE server_id=? AND channel_id=? \
                 ORDER BY alias COLLATE NOCASE",
            )
            .bind(server_id)
            .bind(&channel_id)
            .fetch_all(&mut *tx)
            .await?;
            let override_rows = sqlx::query(
                "SELECT target_id,allow_bits,deny_bits FROM channel_permission_overrides \
                 WHERE channel_id=? AND target_type='role' ORDER BY target_id",
            )
            .bind(&channel_id)
            .fetch_all(&mut *tx)
            .await?;
            let role_overrides = override_rows
                .into_iter()
                .map(|override_row| TemplateRoleOverride {
                    role_id: override_row.get(0),
                    allow_bits: override_row.get(1),
                    deny_bits: override_row.get(2),
                })
                .collect();
            channels.push(TemplateChannel {
                id: channel_id,
                name: row.get(1),
                topic: row.get(2),
                category_id: row.get(3),
                position: row.get(4),
                is_private: row.get::<i64, _>(5) != 0,
                channel_type: row.get(6),
                slowmode_seconds: row.get(7),
                is_nsfw: row.get::<i64, _>(8) != 0,
                is_announcement: row.get::<i64, _>(9) != 0,
                is_default: row.get::<i64, _>(10) != 0,
                aliases,
                role_overrides,
            });
        }
        let config = TemplateConfig {
            format_version: 1,
            channels,
            categories,
            roles,
        };
        config.validate()?;
        let template_id = Uuid::new_v4().to_string();
        let created_at: String = sqlx::query_scalar(
            "INSERT INTO server_templates( \
                id,name,description,server_id,created_by,config,format_version \
             ) VALUES(?,?,?,?,?,?,1) RETURNING created_at",
        )
        .bind(&template_id)
        .bind(name)
        .bind(description)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .bind(
            serde_json::to_string(&config)
                .map_err(|_| CommunityError::InvalidInput("invalid server template"))?,
        )
        .fetch_one(&mut *tx)
        .await?;
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut tx,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "server_template_create",
                target_type: Some("server_template"),
                target_id: Some(&template_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(CreatedTemplate {
            id: template_id,
            created_at,
        })
    }

    pub async fn list_templates(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<(Vec<ServerTemplateRow>, AuthorizationStamp), CommunityError> {
        let server_id = server_id.as_str();
        let mut tx = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let rows = sqlx::query_as::<_, ServerTemplateRow>(
            "SELECT * FROM server_templates WHERE server_id=? ORDER BY created_at DESC,id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await?;
        let stamp = self
            .authorization
            .authorization_stamp(&mut tx, server_id, &[])
            .await?;
        tx.commit().await?;
        Ok((rows, stamp))
    }

    pub async fn instantiate_template(
        &self,
        actor: &Actor,
        template_id: &str,
        server_name: &str,
    ) -> Result<ServerId, CommunityError> {
        crate::engine::validation::validate_server_name(server_name)
            .map_err(|_| CommunityError::InvalidInput("invalid server name"))?;
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.auth.validate_actor_in(&mut tx, actor).await?;
        let template: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT server_id,format_version,config FROM server_templates WHERE id=?",
        )
        .bind(template_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (source_server_id, format_version, config_json) =
            template.ok_or(CommunityError::Forbidden)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                &source_server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        if format_version != 1 {
            return Err(CommunityError::InvalidInput(
                "unsupported server template version",
            ));
        }
        let config: TemplateConfig = serde_json::from_str(&config_json)
            .map_err(|_| CommunityError::InvalidInput("invalid server template"))?;
        config.validate()?;
        if config.channels.iter().any(|channel| channel.is_private) {
            self.require_private_template_authority_in(&mut tx, actor, &source_server_id)
                .await?;
        }
        let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM servers WHERE owner_id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&mut *tx)
            .await?;
        if owned >= 100 {
            return Err(CommunityError::InvalidInput("server limit reached"));
        }

        let server_id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES(?,?,?)")
            .bind(&server_id)
            .bind(server_name)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,'owner')")
            .bind(&server_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await?;
        let alias = format!("s-{}", server_id.replace('-', ""));
        sqlx::query("INSERT INTO server_aliases(alias,server_id) VALUES(?,?)")
            .bind(alias)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO user_default_servers(user_id,server_id) VALUES(?,?)")
            .bind(actor.user_id().as_str())
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;

        let mut category_ids = HashMap::new();
        for category in config.categories {
            let new_id = Uuid::new_v4().to_string();
            category_ids.insert(category.id, new_id.clone());
            sqlx::query(
                "INSERT INTO channel_categories(id,server_id,name,position) VALUES(?,?,?,?)",
            )
            .bind(new_id)
            .bind(&server_id)
            .bind(category.name)
            .bind(category.position)
            .execute(&mut *tx)
            .await?;
        }
        let mut role_ids = HashMap::new();
        for role in config.roles {
            let new_id = Uuid::new_v4().to_string();
            role_ids.insert(role.id, new_id.clone());
            sqlx::query(
                "INSERT INTO roles(id,server_id,name,color,position,permissions,is_default) \
                 VALUES(?,?,?,?,?,?,?)",
            )
            .bind(new_id)
            .bind(&server_id)
            .bind(role.name)
            .bind(role.color)
            .bind(role.position)
            .bind(role.permissions)
            .bind(i64::from(role.is_default))
            .execute(&mut *tx)
            .await?;
        }
        for channel in config.channels {
            let category_id = channel
                .category_id
                .as_ref()
                .and_then(|id| category_ids.get(id));
            let channel_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO channels( \
                    id,server_id,name,topic,category_id,position,is_private,channel_type, \
                    slowmode_seconds,is_nsfw,is_announcement,is_default \
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&channel_id)
            .bind(&server_id)
            .bind(channel.name)
            .bind(channel.topic)
            .bind(category_id)
            .bind(channel.position)
            .bind(i64::from(channel.is_private))
            .bind(channel.channel_type)
            .bind(channel.slowmode_seconds)
            .bind(i64::from(channel.is_nsfw))
            .bind(i64::from(channel.is_announcement))
            .bind(i64::from(channel.is_default))
            .execute(&mut *tx)
            .await?;
            for alias in channel.aliases {
                sqlx::query(
                    "INSERT INTO channel_aliases(server_id,alias,channel_id) VALUES(?,?,?)",
                )
                .bind(&server_id)
                .bind(alias)
                .bind(&channel_id)
                .execute(&mut *tx)
                .await?;
            }
            for rule in channel.role_overrides {
                let target_role_id = role_ids
                    .get(&rule.role_id)
                    .ok_or(CommunityError::InvalidInput("invalid server template"))?;
                sqlx::query(
                    "INSERT INTO channel_permission_overrides( \
                        id,channel_id,target_type,target_id,allow_bits,deny_bits \
                     ) VALUES(?,?,'role',?,?,?)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(&channel_id)
                .bind(target_role_id)
                .bind(rule.allow_bits)
                .bind(rule.deny_bits)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query("UPDATE server_templates SET use_count=use_count+1 WHERE id=?")
            .bind(template_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(ServerId::parse(server_id).expect("UUID server IDs satisfy the resource ID boundary"))
    }

    pub async fn delete_template(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        template_id: &str,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let deleted = sqlx::query("DELETE FROM server_templates WHERE id=? AND server_id=?")
            .bind(template_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut tx,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "server_template_delete",
                target_type: Some("server_template"),
                target_id: Some(template_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn redeem_invite(
        &self,
        actor: &Actor,
        code: &str,
    ) -> Result<RedeemedInvite, CommunityError> {
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(CommunityError::from)?;
        let invite: Option<(String, String)> = sqlx::query_as(
            "SELECT id,server_id FROM invites WHERE code=? \
             AND (expires_at IS NULL OR julianday(expires_at)>julianday('now'))",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let (invite_id, server_id) = invite.ok_or(CommunityError::InvalidInput(
            "invalid or expired invite code",
        ))?;
        let banned: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bans WHERE server_id=? AND user_id=?)")
                .bind(&server_id)
                .bind(actor.user_id().as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(CommunityError::from)?;
        if banned {
            return Err(CommunityError::Forbidden);
        }
        let already_member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
        )
        .bind(&server_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        if !already_member {
            let used = sqlx::query(
                "UPDATE invites SET use_count=use_count+1 WHERE id=? \
                 AND (max_uses IS NULL OR use_count<max_uses) \
                 AND NOT EXISTS(SELECT 1 FROM bans WHERE server_id=? AND user_id=?)",
            )
            .bind(&invite_id)
            .bind(&server_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await
            .map_err(CommunityError::from)?;
            if used.rows_affected() != 1 {
                return Err(CommunityError::Forbidden);
            }
            sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,'member')")
                .bind(&server_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *tx)
                .await
                .map_err(CommunityError::from)?;
        }
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(RedeemedInvite {
            server_id: ServerId::from_stored(server_id)?,
        })
    }

    /// Record acceptance of the server's current rules version.
    ///
    /// The membership check and version read happen in the same admitted write
    /// transaction, so a concurrent rules edit cannot leave a stale version
    /// recorded as current.
    pub async fn accept_rules(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<i64, CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(CommunityError::from)?;
        let accepted_version: Option<i64> = sqlx::query_scalar(
            "UPDATE server_members \
             SET rules_accepted=1,accepted_rules_version=( \
                 SELECT rules_version FROM servers WHERE id=? \
             ) WHERE server_id=? AND user_id=? \
             RETURNING accepted_rules_version",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let accepted_version = accepted_version.ok_or(CommunityError::Forbidden)?;
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(accepted_version)
    }

    pub async fn update_event_status(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        event_id: &str,
        status: &str,
    ) -> Result<ServerEventRow, CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(CommunityError::from)?;
        let row = sqlx::query_as::<_, ServerEventRow>(
            "UPDATE server_events SET status=?,updated_at=datetime('now') \
             WHERE id=? AND server_id=? AND integrity_state='active' \
               AND (status=? \
                    OR (status='scheduled' AND ? IN ('active','cancelled')) \
                    OR (status='active' AND ? IN ('completed','cancelled'))) \
             RETURNING *",
        )
        .bind(status)
        .bind(event_id)
        .bind(server_id)
        .bind(status)
        .bind(status)
        .bind(status)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?
        .ok_or(CommunityError::Forbidden)?;
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(row)
    }

    pub async fn delete_event(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        event_id: &str,
    ) -> Result<(), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut tx,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(CommunityError::from)?;
        let deleted = sqlx::query(
            "DELETE FROM server_events WHERE id=? AND server_id=? AND integrity_state='active'",
        )
        .bind(event_id)
        .bind(server_id)
        .execute(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        if deleted.rows_affected() != 1 {
            return Err(CommunityError::Forbidden);
        }
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(())
    }

    pub async fn set_rsvp(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        event_id: &str,
        status: Option<&str>,
    ) -> Result<(Option<ChannelId>, Vec<EventRsvpRow>), CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        let channel_id: Option<Option<String>> = sqlx::query_scalar(
            "SELECT channel_id FROM server_events WHERE id=? AND server_id=? AND integrity_state='active'",
        )
        .bind(event_id)
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let channel_id = channel_id.ok_or(CommunityError::Forbidden)?;
        if let Some(channel_id) = channel_id.as_deref() {
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await
                .map_err(CommunityError::from)?;
        } else {
            self.authorization
                .require_server_actor_in(
                    &mut tx,
                    &self.auth,
                    actor,
                    server_id,
                    Permissions::VIEW_CHANNELS,
                )
                .await
                .map_err(CommunityError::from)?;
        }
        match status {
            Some(status) => {
                sqlx::query(
                    "INSERT INTO event_rsvps(event_id,user_id,status) VALUES(?,?,?) \
                     ON CONFLICT(event_id,user_id) DO UPDATE SET status=excluded.status",
                )
                .bind(event_id)
                .bind(actor.user_id().as_str())
                .bind(status)
                .execute(&mut *tx)
                .await
                .map_err(CommunityError::from)?;
            }
            None => {
                sqlx::query("DELETE FROM event_rsvps WHERE event_id=? AND user_id=?")
                    .bind(event_id)
                    .bind(actor.user_id().as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(CommunityError::from)?;
            }
        }
        let rows = sqlx::query_as::<_, EventRsvpRow>(
            "SELECT * FROM event_rsvps WHERE event_id=? ORDER BY created_at,user_id",
        )
        .bind(event_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        tx.commit().await.map_err(CommunityError::from)?;
        let channel_id = channel_id.map(ChannelId::from_stored).transpose()?;
        Ok((channel_id, rows))
    }

    pub async fn list_rsvps(
        &self,
        actor: &Actor,
        event_id: &str,
    ) -> Result<(ServerId, Option<ChannelId>, Vec<EventRsvpRow>), CommunityError> {
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        let scope: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT server_id,channel_id FROM server_events WHERE id=? AND integrity_state='active'",
        )
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let (server_id, channel_id) = scope.ok_or(CommunityError::Forbidden)?;
        if let Some(channel_id) = channel_id.as_deref() {
            self.authorization
                .authorize_actor_in(&mut tx, &self.auth, actor, channel_id, ChannelAction::View)
                .await
                .map_err(CommunityError::from)?;
        } else {
            self.authorization
                .require_server_actor_in(
                    &mut tx,
                    &self.auth,
                    actor,
                    &server_id,
                    Permissions::VIEW_CHANNELS,
                )
                .await
                .map_err(CommunityError::from)?;
        }
        let rows = sqlx::query_as::<_, EventRsvpRow>(
            "SELECT * FROM event_rsvps WHERE event_id=? ORDER BY created_at,user_id",
        )
        .bind(event_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        tx.commit().await.map_err(CommunityError::from)?;
        Ok((
            ServerId::from_stored(server_id)?,
            channel_id.map(ChannelId::from_stored).transpose()?,
            rows,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_id(value: &str) -> ServerId {
        ServerId::from_stored(value).unwrap()
    }

    fn channel_id(value: &str) -> ChannelId {
        ChannelId::from_stored(value).unwrap()
    }

    #[tokio::test]
    async fn invite_deletion_is_server_scoped_and_revalidates_actor() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES('a','A','owner'),('b','B','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('a','owner','owner'),('b','owner','owner')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO invites(id,server_id,code,created_by) VALUES('invite-a','a','code-a','owner'),('invite-b','b','code-b','owner')").execute(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth.clone(),
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        assert!(matches!(
            service
                .delete_invite(&actor, &server_id("a"), "invite-b")
                .await,
            Err(CommunityError::Forbidden)
        ));
        auth.revoke_credential(actor.credential_id()).await.unwrap();
        assert!(matches!(
            service
                .delete_invite(&actor, &server_id("a"), "invite-a")
                .await,
            Err(CommunityError::Authentication(_))
        ));
        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM invites WHERE id IN ('invite-a','invite-b')")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 2);
    }

    #[tokio::test]
    async fn vanity_code_is_scoped_unique_audited_and_revalidates_actor() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('other','other')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id,vanity_code) VALUES \
             ('a','A','owner',NULL),('b','B','other','taken')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('a','owner','owner'),('b','other','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth.clone(),
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );

        assert!(matches!(
            service
                .set_vanity_code(&actor, &server_id("b"), Some("unavailable"))
                .await,
            Err(CommunityError::Forbidden)
        ));
        assert!(matches!(
            service
                .set_vanity_code(&actor, &server_id("a"), Some("taken"))
                .await,
            Err(CommunityError::Conflict("vanity code unavailable"))
        ));
        service
            .set_vanity_code(&actor, &server_id("a"), Some("available"))
            .await
            .unwrap();
        let persisted: (Option<String>, i64) = sqlx::query_as(
            "SELECT vanity_code,(SELECT count(*) FROM audit_log \
             WHERE server_id='a' AND actor_id='owner' \
             AND action_type='server_vanity_update') FROM servers WHERE id='a'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(persisted, (Some("available".to_owned()), 1));

        auth.revoke_credential(actor.credential_id()).await.unwrap();
        assert!(matches!(
            service
                .set_vanity_code(&actor, &server_id("a"), Some("changed"))
                .await,
            Err(CommunityError::Authentication(_))
        ));
        let persisted: Option<String> =
            sqlx::query_scalar("SELECT vanity_code FROM servers WHERE id='a'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(persisted.as_deref(), Some("available"));
    }

    #[tokio::test]
    async fn competing_announcement_follows_cannot_commit_a_cycle() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES('a','A','owner'),('b','B','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('a','owner','owner'),('b','owner','owner')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,is_announcement) VALUES('ca','a','#a',1),('cb','b','#b',1)").execute(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        let ca = channel_id("ca");
        let cb = channel_id("cb");
        let (left, right) = tokio::join!(
            service.follow_channel(&actor, &ca, &cb),
            service.follow_channel(&actor, &cb, &ca)
        );
        assert_ne!(left.is_ok(), right.is_ok());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_follows")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn source_channel_override_can_deny_announcement_follow() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('manager','manager')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('source','Source','owner'),('target','Target','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('source','manager','member'),('target','manager','member')")
            .execute(&pool)
            .await
            .unwrap();
        let manager_permissions =
            (Permissions::MANAGE_MESSAGES | Permissions::MANAGE_CHANNELS).bits() as i64;
        sqlx::query("INSERT INTO roles(id,server_id,name,permissions) VALUES('source-manager','source','Manager',?),('target-manager','target','Manager',?)")
            .bind(manager_permissions)
            .bind(manager_permissions)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO user_roles(user_id,server_id,role_id) VALUES('manager','source','source-manager'),('manager','target','target-manager')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,is_announcement) VALUES('announcement','source','#announcement',1),('destination','target','#destination',0)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('deny-manager','announcement','role','source-manager',?)")
            .bind(Permissions::MANAGE_MESSAGES.bits() as i64)
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("manager").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );

        assert!(matches!(
            service
                .follow_channel(
                    &actor,
                    &channel_id("announcement"),
                    &channel_id("destination"),
                )
                .await,
            Err(CommunityError::Forbidden)
        ));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_follows")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn banned_existing_member_cannot_redeem_invite_idempotently() {
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
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO invites(id,server_id,code,created_by) VALUES('invite','server','code','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','member','owner')")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("member").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );

        assert!(matches!(
            service.redeem_invite(&actor, "code").await,
            Err(CommunityError::Forbidden)
        ));
        let uses: i64 = sqlx::query_scalar("SELECT use_count FROM invites WHERE id='invite'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(uses, 0);
    }

    #[tokio::test]
    async fn final_invite_use_is_single_winner_and_failed_membership_rolls_back() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('first','first'),('second','second'),('broken','broken')")
            .execute(&pool).await.unwrap();
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
        sqlx::query("INSERT INTO invites(id,server_id,code,created_by,max_uses) VALUES('last','server','last-code','owner',1),('broken-invite','server','broken-code','owner',1)")
            .execute(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, first) = auth.issue_web_session("first").await.unwrap();
        let (_, second) = auth.issue_web_session("second").await.unwrap();
        let (_, broken) = auth.issue_web_session("broken").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );

        let (first_result, second_result) = tokio::join!(
            service.redeem_invite(&first, "last-code"),
            service.redeem_invite(&second, "last-code"),
        );
        assert_eq!(
            usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
            1
        );
        let use_count: i64 = sqlx::query_scalar("SELECT use_count FROM invites WHERE id='last'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let members: i64 = sqlx::query_scalar("SELECT count(*) FROM server_members WHERE server_id='server' AND user_id IN ('first','second')").fetch_one(&pool).await.unwrap();
        assert_eq!((use_count, members), (1, 1));

        sqlx::query("CREATE TRIGGER reject_broken_member BEFORE INSERT ON server_members WHEN NEW.user_id='broken' BEGIN SELECT RAISE(ABORT,'fixture membership failure'); END")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            service.redeem_invite(&broken, "broken-code").await,
            Err(CommunityError::Database(_))
        ));
        let use_count: i64 =
            sqlx::query_scalar("SELECT use_count FROM invites WHERE id='broken-invite'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(use_count, 0);
    }

    #[tokio::test]
    async fn rules_acceptance_tracks_the_current_version_and_requires_membership() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username) VALUES \
             ('owner','owner'),('member','member'),('outsider','outsider')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id,rules_text) VALUES('server','Server','owner','v1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, member) = auth.issue_web_session("member").await.unwrap();
        let (_, outsider) = auth.issue_web_session("outsider").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );

        assert_eq!(
            service
                .accept_rules(&member, &server_id("server"))
                .await
                .unwrap(),
            1
        );
        let (_, accepted, _) = service
            .get_community(&member, &server_id("server"))
            .await
            .unwrap();
        assert!(accepted);
        assert!(
            crate::db::queries::community::has_accepted_rules(&pool, "server", "member")
                .await
                .unwrap()
        );

        sqlx::query("UPDATE servers SET rules_text='v2' WHERE id='server'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !crate::db::queries::community::has_accepted_rules(&pool, "server", "member")
                .await
                .unwrap()
        );
        let (_, accepted, _) = service
            .get_community(&member, &server_id("server"))
            .await
            .unwrap();
        assert!(!accepted);
        assert_eq!(
            service
                .accept_rules(&member, &server_id("server"))
                .await
                .unwrap(),
            2
        );
        assert!(matches!(
            service.accept_rules(&outsider, &server_id("server")).await,
            Err(CommunityError::Forbidden)
        ));
    }

    #[tokio::test]
    async fn templates_are_admin_scoped_coherent_snapshots_and_never_copy_user_grants() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users(id,username) VALUES \
             ('owner','owner'),('manager','manager'),('member','member'),('outsider','outsider')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('source','Source','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('source','owner','owner'),('source','manager','member'),('source','member','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO roles(id,server_id,name,position,permissions,is_default) VALUES \
             ('everyone','source','@everyone',0,?,1), \
             ('private-role','source','Private',1,0,0), \
             ('manager-role','source','Manager',2,?,0)",
        )
        .bind(Permissions::VIEW_CHANNELS.bits() as i64)
        .bind((Permissions::VIEW_CHANNELS | Permissions::MANAGE_SERVER).bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_roles(server_id,user_id,role_id) \
             VALUES('source','manager','manager-role')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,topic,is_default,position) VALUES \
             ('general','source','#general','Public',1,0), \
             ('secret','source','#secret','Hidden topic',0,1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE channels SET is_private=1 WHERE id='secret'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT OR IGNORE INTO channel_aliases(server_id,alias,channel_id) VALUES \
             ('source','home','general'),('source','vault','secret')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_permission_overrides( \
                id,channel_id,target_type,target_id,allow_bits,deny_bits \
             ) VALUES \
             ('role-override','secret','role','private-role',?,0), \
             ('manager-deny','secret','role','manager-role',0,?), \
             ('user-override','secret','user','member',?,0)",
        )
        .bind(Permissions::VIEW_CHANNELS.bits() as i64)
        .bind(Permissions::VIEW_CHANNELS.bits() as i64)
        .bind(Permissions::VIEW_CHANNELS.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, owner) = auth.issue_web_session("owner").await.unwrap();
        let (_, manager) = auth.issue_web_session("manager").await.unwrap();
        let (_, member) = auth.issue_web_session("member").await.unwrap();
        let (_, outsider) = auth.issue_web_session("outsider").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );
        assert!(matches!(
            service
                .create_template(
                    &manager,
                    &server_id("source"),
                    "Must not expose private config",
                    None,
                )
                .await,
            Err(CommunityError::Forbidden)
        ));
        let created = service
            .create_template(&owner, &server_id("source"), "Private admin template", None)
            .await
            .unwrap();
        let config_json: String =
            sqlx::query_scalar("SELECT config FROM server_templates WHERE id=?")
                .bind(&created.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let config: TemplateConfig = serde_json::from_str(&config_json).unwrap();
        let secret = config
            .channels
            .iter()
            .find(|channel| channel.id == "secret")
            .unwrap();
        assert_eq!(secret.topic, "Hidden topic");
        assert_eq!(secret.aliases, vec!["vault"]);
        assert_eq!(secret.role_overrides.len(), 2);
        assert!(
            secret
                .role_overrides
                .iter()
                .any(|rule| rule.role_id == "private-role")
        );
        assert!(
            secret
                .role_overrides
                .iter()
                .any(|rule| rule.role_id == "manager-role")
        );
        assert!(matches!(
            service
                .instantiate_template(&manager, &created.id, "Leaked")
                .await,
            Err(CommunityError::Forbidden)
        ));
        assert!(matches!(
            service
                .instantiate_template(&member, &created.id, "Leaked")
                .await,
            Err(CommunityError::Forbidden)
        ));
        assert!(matches!(
            service
                .instantiate_template(&outsider, &created.id, "Leaked")
                .await,
            Err(CommunityError::Forbidden)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM servers WHERE name='Leaked'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM audit_log WHERE action_type='server_template_create' \
                 AND target_id=? AND actor_id='owner'",
            )
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        service
            .delete_template(&owner, &server_id("source"), &created.id)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM audit_log WHERE action_type='server_template_delete' \
                 AND target_id=? AND actor_id='owner'",
            )
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM server_templates WHERE id=?")
                .bind(&created.id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn template_instantiation_remaps_ids_atomically_and_rejects_legacy_formats() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('source','Source','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('source','owner','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let config = serde_json::json!({
            "format_version": 1,
            "categories": [{"id":"old-category","name":"Info","position":0}],
            "roles": [{"id":"old-everyone","name":"@everyone","color":null,"position":0,
                "permissions": Permissions::VIEW_CHANNELS.bits(), "is_default":true}],
            "channels": [{"id":"old-channel","name":"#welcome","topic":"Welcome",
                "category_id":"old-category","position":0,"is_private":false,
                "channel_type":"text","slowmode_seconds":0,"is_nsfw":false,
                "is_announcement":true,"is_default":true,"aliases":["welcome","start"],
                "role_overrides":[{"role_id":"old-everyone","allow_bits":0,
                    "deny_bits":Permissions::SEND_MESSAGES.bits()}]}]
        });
        sqlx::query("INSERT INTO server_templates(id,name,server_id,created_by,config,format_version) VALUES('template','Template','source','owner',?,1)")
            .bind(config.to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_templates(id,name,server_id,created_by,config,format_version) VALUES('legacy','Legacy','source','owner','{}',0)")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool.clone()),
        );

        let new_server = service
            .instantiate_template(&actor, "template", "Copy")
            .await
            .unwrap();
        assert_ne!(new_server.as_str(), "source");
        let copied: (String, String, String) = sqlx::query_as(
            "SELECT c.id,c.category_id,cc.id FROM channels c JOIN channel_categories cc ON cc.id=c.category_id WHERE c.server_id=?",
        )
        .bind(new_server.as_str())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_ne!(copied.0, "old-channel");
        assert_ne!(copied.1, "old-category");
        assert_eq!(copied.1, copied.2);
        let remapped: (i64, i64, String, String, i64) = sqlx::query_as(
            "SELECT c.is_default,c.is_announcement,a.channel_id,o.target_id,o.deny_bits \
             FROM channels c JOIN channel_aliases a ON a.channel_id=c.id AND a.alias='start' \
             JOIN channel_permission_overrides o ON o.channel_id=c.id AND o.target_type='role' \
             JOIN roles r ON r.id=o.target_id AND r.server_id=c.server_id \
             WHERE c.server_id=?",
        )
        .bind(new_server.as_str())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remapped.0, 1);
        assert_eq!(remapped.1, 1);
        assert_eq!(remapped.2, copied.0);
        assert_ne!(remapped.3, "old-everyone");
        assert_eq!(remapped.4, Permissions::SEND_MESSAGES.bits() as i64);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT use_count FROM server_templates WHERE id='template'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM servers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(matches!(
            service
                .instantiate_template(&actor, "legacy", "Must Not Exist")
                .await,
            Err(CommunityError::InvalidInput(_))
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM servers")
                .fetch_one(&pool)
                .await
                .unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn scheduled_event_status_transitions_are_monotonic() {
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
        sqlx::query("INSERT INTO server_events(id,server_id,name,start_time,created_by,integrity_state) VALUES('event','server','Event','2030-01-01T00:00:00Z','owner','active')")
            .execute(&pool)
            .await
            .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let service = CommunityService::new(
            pool.clone(),
            auth,
            super::super::write_admission::WriteAdmission::new(pool),
        );

        assert!(matches!(
            service
                .update_event_status(&actor, &server_id("server"), "event", "completed")
                .await,
            Err(CommunityError::Forbidden)
        ));
        assert_eq!(
            service
                .update_event_status(&actor, &server_id("server"), "event", "active")
                .await
                .unwrap()
                .status,
            "active"
        );
        assert!(matches!(
            service
                .update_event_status(&actor, &server_id("server"), "event", "scheduled")
                .await,
            Err(CommunityError::Forbidden)
        ));
        assert_eq!(
            service
                .update_event_status(&actor, &server_id("server"), "event", "completed")
                .await
                .unwrap()
                .status,
            "completed"
        );
    }
}
