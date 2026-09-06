use crate::auth::authority::AuthError;
use crate::auth::authority::{Actor, AuthService};
use crate::engine::authorization::AuthorizationError;
use crate::engine::authorization::AuthorizationService;
use crate::engine::ids::{ChannelId, ServerId};
use crate::engine::permissions::Permissions;
use crate::engine::write_admission::WriteAdmissionError;
use chrono::Utc;
use sqlx::{Row, SqliteConnection, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

pub struct CreateAutomodRule<'a> {
    pub server_id: &'a ServerId,
    pub name: &'a str,
    pub rule_type: &'a str,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

pub struct UpdateAutomodRule<'a> {
    pub server_id: &'a ServerId,
    pub rule_id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub config: &'a str,
    pub action_type: &'a str,
    pub timeout_duration_seconds: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModerationError {
    #[error("{0}")]
    Validation(String),
    #[error("moderation authentication failed")]
    Unauthenticated,
    #[error("moderation authorization failed")]
    Authorization(#[from] AuthorizationError),
    #[error("moderation write admission failed")]
    Admission(#[from] WriteAdmissionError),
    #[error("moderation database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("moderation dependency unavailable")]
    DependencyUnavailable,
    #[error("resource unavailable")]
    Unavailable,
}

impl ModerationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "INVALID_INPUT",
            Self::Unauthenticated
            | Self::Authorization(AuthorizationError::Authentication(
                AuthError::Invalid
                | AuthError::Revoked
                | AuthError::Expired
                | AuthError::Disabled
                | AuthError::Token(_),
            )) => "UNAUTHENTICATED",
            Self::Authorization(AuthorizationError::Authentication(
                AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_)
            | Self::DependencyUnavailable => "DEPENDENCY_UNAVAILABLE",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "RESOURCE_UNAVAILABLE"
            }
        }
    }

    pub fn safe_message(&self) -> &str {
        match self {
            Self::Validation(message) => message,
            Self::Unauthenticated
            | Self::Authorization(AuthorizationError::Authentication(
                AuthError::Invalid
                | AuthError::Revoked
                | AuthError::Expired
                | AuthError::Disabled
                | AuthError::Token(_),
            )) => "authentication required",
            Self::Authorization(AuthorizationError::Authentication(
                AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_)
            | Self::DependencyUnavailable => "dependency unavailable",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "resource unavailable"
            }
        }
    }

    pub fn wire_message(&self) -> String {
        format!("{}: {}", self.code(), self.safe_message())
    }
}

#[derive(Clone)]
pub struct ModerationService {
    auth: AuthService,
    authorization: AuthorizationService,
    writes: super::write_admission::WriteAdmission,
}

impl ModerationService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            auth,
            writes,
        }
    }

    async fn check_member_hierarchy_in(
        connection: &mut SqliteConnection,
        server_id: &str,
        actor_user_id: &str,
        target_user_id: &str,
    ) -> Result<(), ModerationError> {
        let owner_id: Option<String> = sqlx::query_scalar(
            "SELECT s.owner_id FROM servers s \
             JOIN server_members target ON target.server_id=s.id AND target.user_id=? \
             WHERE s.id=?",
        )
        .bind(target_user_id)
        .bind(server_id)
        .fetch_optional(&mut *connection)
        .await?;
        let owner_id = owner_id.ok_or(ModerationError::Unavailable)?;
        if target_user_id == owner_id {
            return Err(ModerationError::Unavailable);
        }
        if actor_user_id == owner_id {
            return Ok(());
        }
        let actor_position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(r.position),0) FROM user_roles ur \
             JOIN roles r ON r.id=ur.role_id AND r.server_id=ur.server_id \
             WHERE ur.server_id=? AND ur.user_id=?",
        )
        .bind(server_id)
        .bind(actor_user_id)
        .fetch_one(&mut *connection)
        .await?;
        let target_position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(r.position),0) FROM user_roles ur \
             JOIN roles r ON r.id=ur.role_id AND r.server_id=ur.server_id \
             WHERE ur.server_id=? AND ur.user_id=?",
        )
        .bind(server_id)
        .bind(target_user_id)
        .fetch_one(&mut *connection)
        .await?;
        if actor_position <= target_position {
            return Err(ModerationError::Unavailable);
        }
        Ok(())
    }

    fn validate_reason(reason: Option<&str>) -> Result<(), ModerationError> {
        if reason.is_some_and(|value| {
            value.len() > 512 || value.chars().any(|character| character == '\0')
        }) {
            return Err(ModerationError::Validation(
                "moderation reason must contain at most 512 bytes".into(),
            ));
        }
        Ok(())
    }

    pub async fn kick_member(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        reason: Option<&str>,
        channel_id: Option<&ChannelId>,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.map(ChannelId::as_str);
        Self::validate_reason(reason)?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        if let Some(channel_id) = channel_id {
            self.authorization
                .require_channel_actor_permission_in(
                    &mut transaction,
                    &self.auth,
                    actor,
                    server_id,
                    channel_id,
                    Permissions::KICK_MEMBERS,
                )
                .await?;
        } else {
            self.authorization
                .require_server_actor_in(
                    &mut transaction,
                    &self.auth,
                    actor,
                    server_id,
                    Permissions::KICK_MEMBERS,
                )
                .await?;
        }
        Self::check_member_hierarchy_in(
            &mut transaction,
            server_id,
            actor.user_id().as_str(),
            target_user_id,
        )
        .await?;
        let removed = sqlx::query("DELETE FROM server_members WHERE server_id=? AND user_id=?")
            .bind(server_id)
            .bind(target_user_id)
            .execute(&mut *transaction)
            .await?;
        if removed.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "member_kick",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn ban_member(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        reason: Option<&str>,
        delete_message_days: i32,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        Self::validate_reason(reason)?;
        if !(0..=7).contains(&delete_message_days) {
            return Err(ModerationError::Validation(
                "delete_message_days must be between 0 and 7".into(),
            ));
        }
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::BAN_MEMBERS,
            )
            .await?;
        Self::check_member_hierarchy_in(
            &mut transaction,
            server_id,
            actor.user_id().as_str(),
            target_user_id,
        )
        .await?;
        let ban_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO bans(id,server_id,user_id,banned_by,reason,delete_message_days) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&ban_id)
        .bind(server_id)
        .bind(target_user_id)
        .bind(actor.user_id().as_str())
        .bind(reason)
        .bind(delete_message_days)
        .execute(&mut *transaction)
        .await?;
        let removed = sqlx::query("DELETE FROM server_members WHERE server_id=? AND user_id=?")
            .bind(server_id)
            .bind(target_user_id)
            .execute(&mut *transaction)
            .await?;
        if removed.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let cleanup_scheduled = delete_message_days > 0;
        if cleanup_scheduled {
            let cleanup_job_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO moderation_cleanup_jobs( \
                    id,ban_id,server_id,user_id,actor_id,cutoff_at \
                 ) VALUES(?,?,?,?,?,datetime('now',?))",
            )
            .bind(&cleanup_job_id)
            .bind(&ban_id)
            .bind(server_id)
            .bind(target_user_id)
            .bind(actor.user_id().as_str())
            .bind(format!("-{delete_message_days} days"))
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO moderation_cleanup_scopes(job_id,conversation_id,through_sequence) \
                 SELECT ?,cv.id,MAX(cv.next_message_sequence-1,0) \
                 FROM conversations cv JOIN channels c ON c.id=cv.channel_id \
                 WHERE c.server_id=?",
            )
            .bind(&cleanup_job_id)
            .bind(server_id)
            .execute(&mut *transaction)
            .await?;
        }
        let changes = serde_json::json!({
            "delete_message_days": delete_message_days,
            "cleanup_scheduled": cleanup_scheduled,
        })
        .to_string();
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "member_ban",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason,
                changes: Some(&changes),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn unban_member(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::BAN_MEMBERS,
            )
            .await?;
        let removed = sqlx::query("DELETE FROM bans WHERE server_id=? AND user_id=?")
            .bind(server_id)
            .bind(target_user_id)
            .execute(&mut *transaction)
            .await?;
        if removed.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "member_unban",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn timeout_member(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        target_user_id: &str,
        timeout_until: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        Self::validate_reason(reason)?;
        if let Some(timeout_until) = timeout_until {
            let parsed = chrono::DateTime::parse_from_rfc3339(timeout_until).map_err(|_| {
                ModerationError::Validation("timeout_until must be an RFC 3339 timestamp".into())
            })?;
            if parsed.offset().local_minus_utc() != 0 {
                return Err(ModerationError::Validation(
                    "timeout_until must use UTC".into(),
                ));
            }
            let parsed = parsed.with_timezone(&Utc);
            if parsed <= Utc::now() || parsed > Utc::now() + chrono::Duration::days(28) {
                return Err(ModerationError::Validation(
                    "timeout_until must be in the next 28 days".into(),
                ));
            }
        }
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::KICK_MEMBERS,
            )
            .await?;
        Self::check_member_hierarchy_in(
            &mut transaction,
            server_id,
            actor.user_id().as_str(),
            target_user_id,
        )
        .await?;
        let updated = sqlx::query(
            "UPDATE server_members SET timeout_until=? WHERE server_id=? AND user_id=?",
        )
        .bind(timeout_until)
        .bind(server_id)
        .bind(target_user_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let changes = serde_json::json!({"timeout_until": timeout_until}).to_string();
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "member_timeout",
                target_type: Some("user"),
                target_id: Some(target_user_id),
                reason,
                changes: Some(&changes),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn set_slowmode(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        seconds: i32,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if !(0..=21_600).contains(&seconds) {
            return Err(ModerationError::Validation(
                "slow mode must be between 0 and 21600 seconds".into(),
            ));
        }
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let updated =
            sqlx::query("UPDATE channels SET slowmode_seconds=? WHERE id=? AND server_id=?")
                .bind(seconds)
                .bind(channel_id)
                .bind(server_id)
                .execute(&mut *transaction)
                .await?;
        if updated.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let changes = serde_json::json!({"slowmode_seconds": seconds}).to_string();
        self.audit_channel_change_in(
            &mut transaction,
            actor,
            server_id,
            channel_id,
            "channel_slowmode_update",
            &changes,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn set_nsfw(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        is_nsfw: bool,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        let updated = sqlx::query("UPDATE channels SET is_nsfw=? WHERE id=? AND server_id=?")
            .bind(is_nsfw)
            .bind(channel_id)
            .bind(server_id)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let changes = serde_json::json!({"is_nsfw": is_nsfw}).to_string();
        self.audit_channel_change_in(
            &mut transaction,
            actor,
            server_id,
            channel_id,
            "channel_nsfw_update",
            &changes,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn audit_channel_change_in(
        &self,
        transaction: &mut SqliteConnection,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
        action_type: &str,
        changes: &str,
    ) -> Result<(), ModerationError> {
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type,
                target_type: Some("channel"),
                target_id: Some(channel_id),
                reason: None,
                changes: Some(changes),
            },
        )
        .await?;
        Ok(())
    }

    pub async fn bulk_delete_messages(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        channel_id: &ChannelId,
        message_ids: &[String],
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        let channel_id = channel_id.as_str();
        if message_ids.is_empty() {
            return Err(ModerationError::Validation(
                "bulk delete requires at least one message".into(),
            ));
        }
        if message_ids.len() > 100 {
            return Err(ModerationError::Validation(
                "bulk delete accepts at most 100 messages".into(),
            ));
        }
        let unique: HashSet<&str> = message_ids.iter().map(String::as_str).collect();
        if unique.len() != message_ids.len() {
            return Err(ModerationError::Validation(
                "message IDs must be unique".into(),
            ));
        }
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                channel_id,
                Permissions::MANAGE_MESSAGES,
            )
            .await?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let mut scoped = sqlx::QueryBuilder::new("SELECT id FROM messages WHERE channel_id=");
        scoped.push_bind(channel_id);
        scoped.push(" AND server_id=");
        scoped.push_bind(server_id);
        scoped.push(" AND deleted_at IS NULL AND id IN (");
        let mut ids = scoped.separated(",");
        for message_id in message_ids {
            ids.push_bind(message_id);
        }
        ids.push_unseparated(")");
        let rows = scoped.build().fetch_all(&mut *transaction).await?;
        if rows.len() != message_ids.len() {
            return Err(ModerationError::Unavailable);
        }
        for row in rows {
            let message_id: String = row.get(0);
            super::messaging::tombstone_moderated_message_in(
                &mut transaction,
                &generation,
                &message_id,
                actor.user_id().as_str(),
            )
            .await
            .map_err(|error| match error {
                super::messaging::MessagingError::Internal(error) => {
                    ModerationError::Database(error)
                }
                super::messaging::MessagingError::DependencyUnavailable => {
                    ModerationError::DependencyUnavailable
                }
                _ => ModerationError::Unavailable,
            })?
            .ok_or(ModerationError::Unavailable)?;
        }
        let changes = serde_json::json!({"message_ids": message_ids}).to_string();
        self.audit_channel_change_in(
            &mut transaction,
            actor,
            server_id,
            channel_id,
            "message_bulk_delete",
            &changes,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_bans(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<Vec<crate::db::models::BanRow>, ModerationError> {
        let server_id = server_id.as_str();
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::BAN_MEMBERS,
            )
            .await?;
        let rows = sqlx::query_as::<_, crate::db::models::BanRow>(
            "SELECT * FROM bans WHERE server_id=? ORDER BY created_at DESC",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows)
    }

    pub async fn list_audit_log(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        action_type: Option<&str>,
        limit: i64,
        before: Option<&str>,
    ) -> Result<Vec<crate::db::models::AuditLogRow>, ModerationError> {
        let server_id = server_id.as_str();
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let limit = limit.clamp(1, 100);
        let rows = match (action_type, before) {
            (Some(action_type), Some(before)) => sqlx::query_as::<_, crate::db::models::AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id=? AND action_type=? AND created_at<? ORDER BY created_at DESC LIMIT ?",
            ).bind(server_id).bind(action_type).bind(before).bind(limit).fetch_all(&mut *transaction).await?,
            (Some(action_type), None) => sqlx::query_as::<_, crate::db::models::AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id=? AND action_type=? ORDER BY created_at DESC LIMIT ?",
            ).bind(server_id).bind(action_type).bind(limit).fetch_all(&mut *transaction).await?,
            (None, Some(before)) => sqlx::query_as::<_, crate::db::models::AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id=? AND created_at<? ORDER BY created_at DESC LIMIT ?",
            ).bind(server_id).bind(before).bind(limit).fetch_all(&mut *transaction).await?,
            (None, None) => sqlx::query_as::<_, crate::db::models::AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id=? ORDER BY created_at DESC LIMIT ?",
            ).bind(server_id).bind(limit).fetch_all(&mut *transaction).await?,
        };
        transaction.commit().await?;
        Ok(rows)
    }

    pub async fn create_automod_rule(
        &self,
        actor: &Actor,
        params: &CreateAutomodRule<'_>,
    ) -> Result<String, ModerationError> {
        let server_id = params.server_id.as_str();
        validate_automod_rule(
            params.name,
            params.rule_type,
            params.config,
            params.action_type,
            params.timeout_duration_seconds,
        )?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let existing_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM automod_rules WHERE server_id=?")
                .bind(server_id)
                .fetch_one(&mut *transaction)
                .await?;
        if existing_count >= 100 {
            return Err(ModerationError::Validation(
                "AutoMod rule limit reached".into(),
            ));
        }
        let rule_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO automod_rules( \
                id,server_id,name,rule_type,config,action_type,timeout_duration_seconds \
             ) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&rule_id)
        .bind(server_id)
        .bind(params.name)
        .bind(params.rule_type)
        .bind(params.config)
        .bind(params.action_type)
        .bind(params.timeout_duration_seconds)
        .execute(&mut *transaction)
        .await?;
        let changes = serde_json::json!({
            "rule_type": params.rule_type,
            "action_type": params.action_type,
            "timeout_duration_seconds": params.timeout_duration_seconds,
        })
        .to_string();
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "automod_rule_create",
                target_type: Some("automod_rule"),
                target_id: Some(&rule_id),
                reason: None,
                changes: Some(&changes),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(rule_id)
    }

    pub async fn update_automod_rule(
        &self,
        actor: &Actor,
        params: &UpdateAutomodRule<'_>,
    ) -> Result<String, ModerationError> {
        let server_id = params.server_id.as_str();
        if params.name.trim().is_empty() || params.name.len() > 100 {
            return Err(ModerationError::Validation(
                "AutoMod rule name must contain 1 to 100 bytes".into(),
            ));
        }
        validate_automod_action(params.action_type, params.timeout_duration_seconds)?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let existing = sqlx::query_as::<_, crate::db::models::AutomodRuleRow>(
            "SELECT * FROM automod_rules WHERE id=? AND server_id=?",
        )
        .bind(params.rule_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ModerationError::Unavailable)?;
        validate_automod_config(&existing.rule_type, params.config)?;
        let updated = sqlx::query(
            "UPDATE automod_rules SET name=?,enabled=?,config=?,action_type=?, \
             timeout_duration_seconds=?,updated_at=datetime('now') \
             WHERE id=? AND server_id=?",
        )
        .bind(params.name)
        .bind(params.enabled)
        .bind(params.config)
        .bind(params.action_type)
        .bind(params.timeout_duration_seconds)
        .bind(params.rule_id)
        .bind(server_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let changes = serde_json::json!({
            "enabled": params.enabled,
            "action_type": params.action_type,
            "timeout_duration_seconds": params.timeout_duration_seconds,
        })
        .to_string();
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "automod_rule_update",
                target_type: Some("automod_rule"),
                target_id: Some(params.rule_id),
                reason: None,
                changes: Some(&changes),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(existing.rule_type)
    }

    pub async fn delete_automod_rule(
        &self,
        actor: &Actor,
        server_id: &ServerId,
        rule_id: &str,
    ) -> Result<(), ModerationError> {
        let server_id = server_id.as_str();
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let deleted = sqlx::query("DELETE FROM automod_rules WHERE id=? AND server_id=?")
            .bind(rule_id)
            .bind(server_id)
            .execute(&mut *transaction)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(ModerationError::Unavailable);
        }
        let audit_id = Uuid::new_v4().to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &crate::db::models::CreateAuditLogParams {
                id: &audit_id,
                server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "automod_rule_delete",
                target_type: Some("automod_rule"),
                target_id: Some(rule_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_automod_rules(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<Vec<crate::db::models::AutomodRuleRow>, ModerationError> {
        let server_id = server_id.as_str();
        let (_permit, mut transaction) = self.writes.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                server_id,
                Permissions::MANAGE_SERVER,
            )
            .await?;
        let rows = sqlx::query_as::<_, crate::db::models::AutomodRuleRow>(
            "SELECT * FROM automod_rules WHERE server_id=? ORDER BY created_at",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows)
    }
}

fn validate_automod_rule(
    name: &str,
    rule_type: &str,
    config: &str,
    action_type: &str,
    timeout_duration_seconds: Option<i32>,
) -> Result<(), ModerationError> {
    if name.trim().is_empty() || name.len() > 100 {
        return Err(ModerationError::Validation(
            "AutoMod rule name must contain 1 to 100 bytes".into(),
        ));
    }
    validate_automod_config(rule_type, config)?;
    validate_automod_action(action_type, timeout_duration_seconds)
}

fn validate_automod_config(rule_type: &str, config: &str) -> Result<(), ModerationError> {
    if config.len() > 64 * 1024 {
        return Err(ModerationError::Validation(
            "AutoMod config exceeds 65536 bytes".into(),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(config)
        .map_err(|_| ModerationError::Validation("Invalid JSON in automod config".into()))?;
    match rule_type {
        "keyword" => {
            let words = parsed
                .get("words")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    ModerationError::Validation("keyword config must have a 'words' array".into())
                })?;
            if words.is_empty() || words.len() > 1_000 {
                return Err(ModerationError::Validation(
                    "keyword config must contain 1 to 1000 entries".into(),
                ));
            }
            if words.iter().any(|word| {
                word.as_str()
                    .is_none_or(|word| word.trim().is_empty() || word.len() > 100)
            }) {
                return Err(ModerationError::Validation(
                    "keyword entries must contain 1 to 100 bytes".into(),
                ));
            }
        }
        "mention_spam" => {
            let maximum = parsed
                .get("max_mentions")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| {
                    ModerationError::Validation(
                        "mention_spam config must have a 'max_mentions' integer".into(),
                    )
                })?;
            if !(1..=100).contains(&maximum) {
                return Err(ModerationError::Validation(
                    "mention_spam 'max_mentions' must be between 1 and 100".into(),
                ));
            }
        }
        "link_filter" => {
            let block_all = parsed.get("block_all");
            let domains = parsed.get("allowed_domains");
            if block_all.is_none() && domains.is_none() {
                return Err(ModerationError::Validation(
                    "link_filter config must have 'block_all' or 'allowed_domains'".into(),
                ));
            }
            if block_all.is_some_and(|value| !value.is_boolean()) {
                return Err(ModerationError::Validation(
                    "link_filter 'block_all' must be a boolean".into(),
                ));
            }
            if let Some(domains) = domains {
                let domains = domains.as_array().ok_or_else(|| {
                    ModerationError::Validation(
                        "link_filter 'allowed_domains' must be an array".into(),
                    )
                })?;
                if domains.len() > 1_000
                    || domains.iter().any(|domain| {
                        domain.as_str().is_none_or(|domain| {
                            domain.is_empty()
                                || domain.len() > 253
                                || domain.contains('/')
                                || domain.contains(':')
                        })
                    })
                {
                    return Err(ModerationError::Validation(
                        "link_filter contains an invalid domain".into(),
                    ));
                }
            }
        }
        _ => {
            return Err(ModerationError::Validation(
                "Invalid rule type. Must be 'keyword', 'mention_spam', or 'link_filter'".into(),
            ));
        }
    }
    Ok(())
}

fn validate_automod_action(
    action_type: &str,
    timeout_duration_seconds: Option<i32>,
) -> Result<(), ModerationError> {
    match (action_type, timeout_duration_seconds) {
        ("timeout", Some(seconds)) if (1..=2_419_200).contains(&seconds) => Ok(()),
        ("timeout", _) => Err(ModerationError::Validation(
            "timeout action requires a duration between 1 and 2419200 seconds".into(),
        )),
        ("delete" | "flag", None) => Ok(()),
        ("delete" | "flag", Some(_)) => Err(ModerationError::Validation(
            "only timeout actions may specify timeout_duration_seconds".into(),
        )),
        _ => Err(ModerationError::Validation(
            "Invalid action type. Must be 'delete', 'timeout', or 'flag'".into(),
        )),
    }
}
