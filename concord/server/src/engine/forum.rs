use std::collections::HashSet;

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService};
use crate::db::models::{CreateAuditLogParams, ForumTagRow};
use crate::engine::authorization::{AuthorizationError, AuthorizationService, ChannelAction};
use crate::engine::events::ForumTagInfo;
use crate::engine::permissions::Permissions;
use crate::engine::write_admission::{WriteAdmission, WriteAdmissionError};

#[derive(Debug, thiserror::Error)]
pub enum ForumError {
    #[error("{0}")]
    Validation(&'static str),
    #[error("forum authorization failed")]
    Authorization(#[from] AuthorizationError),
    #[error("forum write admission failed")]
    Admission(#[from] WriteAdmissionError),
    #[error("forum database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("resource unavailable")]
    Unavailable,
    #[error("thread tag version changed concurrently")]
    Conflict,
}

impl ForumError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "INVALID_INPUT",
            Self::Conflict => "CONFLICT",
            Self::Authorization(AuthorizationError::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_) => "DEPENDENCY_UNAVAILABLE",
            Self::Authorization(AuthorizationError::Authentication(_)) => "UNAUTHENTICATED",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "RESOURCE_UNAVAILABLE"
            }
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Validation(message) => message,
            Self::Conflict => "thread tag version changed concurrently",
            Self::Authorization(AuthorizationError::Authentication(
                crate::auth::authority::AuthError::Database(_)
                | crate::auth::authority::AuthError::VerificationBusy
                | crate::auth::authority::AuthError::HashWorker(_),
            ))
            | Self::Authorization(AuthorizationError::Database(_))
            | Self::Admission(_)
            | Self::Database(_) => "dependency unavailable",
            Self::Authorization(AuthorizationError::Authentication(_)) => "authentication required",
            Self::Authorization(AuthorizationError::Unavailable) | Self::Unavailable => {
                "resource unavailable"
            }
        }
    }

    pub fn wire_message(&self) -> String {
        format!("{}: {}", self.code(), self.safe_message())
    }
}

pub struct CreateForumTag<'a> {
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub name: &'a str,
    pub emoji: Option<&'a str>,
    pub moderated: bool,
}

pub struct UpdateForumTag<'a> {
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub tag_id: &'a str,
    pub name: &'a str,
    pub emoji: Option<&'a str>,
    pub moderated: bool,
    pub position: i32,
}

pub struct ThreadTagMutation {
    pub thread_id: String,
    pub version: i64,
    pub tag_ids: Vec<String>,
}

#[derive(Clone)]
pub struct ForumService {
    pool: SqlitePool,
    writes: WriteAdmission,
}

impl ForumService {
    pub fn new(pool: SqlitePool, writes: WriteAdmission) -> Self {
        Self { pool, writes }
    }

    pub async fn create_tag(
        &self,
        auth: &AuthService,
        actor: &Actor,
        params: CreateForumTag<'_>,
    ) -> Result<ForumTagInfo, ForumError> {
        validate_tag(params.name, params.emoji, None)?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        let authorization = AuthorizationService::new(self.pool.clone());
        authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                params.server_id,
                params.channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        require_forum_channel(&mut transaction, params.server_id, params.channel_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM forum_tags WHERE channel_id=?")
            .bind(params.channel_id)
            .fetch_one(&mut *transaction)
            .await?;
        if count >= 20 {
            return Err(ForumError::Validation("forum tag limit reached"));
        }
        let position: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position),-1)+1 FROM forum_tags WHERE channel_id=?",
        )
        .bind(params.channel_id)
        .fetch_one(&mut *transaction)
        .await?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO forum_tags(id,channel_id,name,emoji,moderated,position) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(params.channel_id)
        .bind(params.name.trim())
        .bind(params.emoji)
        .bind(params.moderated)
        .bind(position)
        .execute(&mut *transaction)
        .await?;
        let changes =
            serde_json::json!({"moderated": params.moderated, "position": position}).to_string();
        insert_audit(
            &mut transaction,
            params.server_id,
            actor,
            "forum_tag_create",
            "forum_tag",
            &id,
            Some(&changes),
        )
        .await?;
        transaction.commit().await?;
        Ok(ForumTagInfo {
            id,
            name: params.name.trim().to_string(),
            emoji: params.emoji.map(str::to_string),
            moderated: params.moderated,
            position,
        })
    }

    pub async fn update_tag(
        &self,
        auth: &AuthService,
        actor: &Actor,
        params: UpdateForumTag<'_>,
    ) -> Result<ForumTagInfo, ForumError> {
        validate_tag(params.name, params.emoji, Some(params.position))?;
        let (_permit, mut transaction) = self.writes.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                params.server_id,
                params.channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        require_forum_channel(&mut transaction, params.server_id, params.channel_id).await?;
        let updated = sqlx::query(
            "UPDATE forum_tags SET name=?,emoji=?,moderated=?,position=? \
             WHERE id=? AND channel_id=?",
        )
        .bind(params.name.trim())
        .bind(params.emoji)
        .bind(params.moderated)
        .bind(params.position)
        .bind(params.tag_id)
        .bind(params.channel_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ForumError::Unavailable);
        }
        let changes = serde_json::json!({
            "moderated": params.moderated,
            "position": params.position,
        })
        .to_string();
        insert_audit(
            &mut transaction,
            params.server_id,
            actor,
            "forum_tag_update",
            "forum_tag",
            params.tag_id,
            Some(&changes),
        )
        .await?;
        transaction.commit().await?;
        Ok(ForumTagInfo {
            id: params.tag_id.to_string(),
            name: params.name.trim().to_string(),
            emoji: params.emoji.map(str::to_string),
            moderated: params.moderated,
            position: params.position,
        })
    }

    pub async fn delete_tag(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
        tag_id: &str,
    ) -> Result<Vec<ThreadTagMutation>, ForumError> {
        let (_permit, mut transaction) = self.writes.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                server_id,
                channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await?;
        require_forum_channel(&mut transaction, server_id, channel_id).await?;
        let affected: Vec<(String, i64, i64, String)> = sqlx::query_as(
            "SELECT c.id,c.thread_tags_version,c.authorization_version,conversations.id \
             FROM thread_tags tt \
             JOIN channels c ON c.id=tt.thread_id \
             JOIN conversations ON conversations.channel_id=c.id \
             WHERE tt.tag_id=? ORDER BY c.id",
        )
        .bind(tag_id)
        .fetch_all(&mut *transaction)
        .await?;
        let deleted = sqlx::query("DELETE FROM forum_tags WHERE id=? AND channel_id=?")
            .bind(tag_id)
            .bind(channel_id)
            .execute(&mut *transaction)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(ForumError::Unavailable);
        }
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let mut mutations = Vec::with_capacity(affected.len());
        for (thread_id, old_version, authorization_version, conversation_id) in affected {
            let tag_ids: Vec<String> = sqlx::query_scalar(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(&thread_id)
            .fetch_all(&mut *transaction)
            .await?;
            let version: i64 = sqlx::query_scalar(
                "UPDATE channels SET thread_tags_version=thread_tags_version+1 \
                 WHERE id=? AND thread_tags_version=? RETURNING thread_tags_version",
            )
            .bind(&thread_id)
            .bind(old_version)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(ForumError::Conflict)?;
            sqlx::query(
                "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
                 VALUES('thread_tags',?,?,datetime('now')) \
                 ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
                    version=excluded.version,updated_at=excluded.updated_at",
            )
            .bind(&thread_id)
            .bind(version)
            .execute(&mut *transaction)
            .await?;
            let descriptor =
                serde_json::json!({"thread_id": thread_id, "tag_ids": tag_ids}).to_string();
            let event_sequence: i64 = sqlx::query_scalar(
                "INSERT INTO event_log( \
                    database_generation,conversation_id,event_kind,entity_type,entity_id, \
                    entity_version,authorization_version,actor_id,descriptor_json \
                 ) VALUES(?,?,'thread_tags_updated','thread_tags',?,?,?,?,?) \
                 RETURNING event_sequence",
            )
            .bind(&generation)
            .bind(conversation_id)
            .bind(&thread_id)
            .bind(version)
            .bind(authorization_version)
            .bind(actor.user_id().as_str())
            .bind(descriptor)
            .fetch_one(&mut *transaction)
            .await?;
            sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
                .bind(event_sequence)
                .execute(&mut *transaction)
                .await?;
            let changes = serde_json::json!({
                "removed_deleted_tag_id": tag_id,
                "new_tag_ids": tag_ids,
                "version": version,
            })
            .to_string();
            insert_audit(
                &mut transaction,
                server_id,
                actor,
                "thread_tags_update",
                "thread",
                &thread_id,
                Some(&changes),
            )
            .await?;
            mutations.push(ThreadTagMutation {
                thread_id,
                version,
                tag_ids,
            });
        }
        insert_audit(
            &mut transaction,
            server_id,
            actor,
            "forum_tag_delete",
            "forum_tag",
            tag_id,
            None,
        )
        .await?;
        transaction.commit().await?;
        Ok(mutations)
    }

    pub async fn list_tags(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
    ) -> Result<Vec<ForumTagInfo>, ForumError> {
        let mut transaction = self.pool.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                channel_id,
                ChannelAction::View,
            )
            .await?;
        require_forum_channel(&mut transaction, server_id, channel_id).await?;
        let rows = sqlx::query_as::<_, ForumTagRow>(
            "SELECT * FROM forum_tags WHERE channel_id=? ORDER BY position,id",
        )
        .bind(channel_id)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows.into_iter().map(row_to_info).collect())
    }

    pub async fn set_thread_tags(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        thread_id: &str,
        tag_ids: Vec<String>,
    ) -> Result<ThreadTagMutation, ForumError> {
        if tag_ids.len() > 5 || tag_ids.iter().collect::<HashSet<_>>().len() != tag_ids.len() {
            return Err(ForumError::Validation(
                "a thread may have up to 5 unique tags",
            ));
        }
        let (_permit, mut transaction) = self.writes.begin().await?;
        let authorization = AuthorizationService::new(self.pool.clone());
        let thread: (String, Option<String>, Option<String>, i64, i64, String) = sqlx::query_as(
            "SELECT channel_type,parent_channel_id,thread_creator_user_id, \
                        thread_tags_version,authorization_version,conversations.id \
                 FROM channels JOIN conversations ON conversations.channel_id=channels.id \
                 WHERE channels.id=? AND channels.server_id=?",
        )
        .bind(thread_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ForumError::Unavailable)?;
        if !matches!(thread.0.as_str(), "public_thread" | "private_thread") {
            return Err(ForumError::Unavailable);
        }
        let parent_channel_id = thread.1.ok_or(ForumError::Unavailable)?;
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                &parent_channel_id,
                ChannelAction::View,
            )
            .await?;
        let can_manage = match authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                server_id,
                &parent_channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await
        {
            Ok(()) => true,
            Err(AuthorizationError::Unavailable) => false,
            Err(error) => return Err(error.into()),
        };
        if thread.2.as_deref() != Some(actor.user_id().as_str()) && !can_manage {
            return Err(ForumError::Unavailable);
        }
        let current: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id")
                .bind(thread_id)
                .fetch_all(&mut *transaction)
                .await?;
        let changed = current
            .iter()
            .chain(tag_ids.iter())
            .filter(|id| current.contains(id) != tag_ids.contains(id))
            .cloned()
            .collect::<HashSet<_>>();
        let all = current
            .iter()
            .chain(tag_ids.iter())
            .cloned()
            .collect::<HashSet<_>>();
        if !all.is_empty() {
            let mut query =
                sqlx::QueryBuilder::new("SELECT id,moderated FROM forum_tags WHERE channel_id=");
            query.push_bind(&parent_channel_id).push(" AND id IN (");
            let mut values = query.separated(",");
            for id in &all {
                values.push_bind(id);
            }
            values.push_unseparated(")");
            let rows = query.build().fetch_all(&mut *transaction).await?;
            if rows.len() != all.len() {
                return Err(ForumError::Unavailable);
            }
            if !can_manage
                && rows.iter().any(|row| {
                    let id: String = row.get(0);
                    changed.contains(&id) && row.get::<i64, _>(1) != 0
                })
            {
                return Err(ForumError::Unavailable);
            }
        }
        sqlx::query("DELETE FROM thread_tags WHERE thread_id=?")
            .bind(thread_id)
            .execute(&mut *transaction)
            .await?;
        for tag_id in &tag_ids {
            sqlx::query("INSERT INTO thread_tags(thread_id,tag_id) VALUES(?,?)")
                .bind(thread_id)
                .bind(tag_id)
                .execute(&mut *transaction)
                .await?;
        }
        let version: i64 = sqlx::query_scalar(
            "UPDATE channels SET thread_tags_version=thread_tags_version+1 \
             WHERE id=? AND thread_tags_version=? RETURNING thread_tags_version",
        )
        .bind(thread_id)
        .bind(thread.3)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ForumError::Conflict)?;
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
             VALUES('thread_tags',?,?,datetime('now')) \
             ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
                version=excluded.version,updated_at=excluded.updated_at",
        )
        .bind(thread_id)
        .bind(version)
        .execute(&mut *transaction)
        .await?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let descriptor = serde_json::json!({"thread_id": thread_id, "tag_ids": tag_ids});
        let event_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'thread_tags_updated','thread_tags',?,?,?,?,?) \
             RETURNING event_sequence",
        )
        .bind(generation)
        .bind(thread.5)
        .bind(thread_id)
        .bind(version)
        .bind(thread.4)
        .bind(actor.user_id().as_str())
        .bind(descriptor.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
            .bind(event_sequence)
            .execute(&mut *transaction)
            .await?;
        let changes = serde_json::json!({
            "old_tag_ids": current,
            "new_tag_ids": tag_ids,
            "version": version,
        })
        .to_string();
        insert_audit(
            &mut transaction,
            server_id,
            actor,
            "thread_tags_update",
            "thread",
            thread_id,
            Some(&changes),
        )
        .await?;
        transaction.commit().await?;
        Ok(ThreadTagMutation {
            thread_id: thread_id.to_string(),
            version,
            tag_ids,
        })
    }

    pub async fn get_thread_tags(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(i64, Vec<String>), ForumError> {
        let mut transaction = self.pool.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                thread_id,
                ChannelAction::View,
            )
            .await?;
        let version: i64 = sqlx::query_scalar(
            "SELECT thread_tags_version FROM channels WHERE id=? AND server_id=? \
             AND channel_type IN ('public_thread','private_thread')",
        )
        .bind(thread_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ForumError::Unavailable)?;
        let tag_ids =
            sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id")
                .bind(thread_id)
                .fetch_all(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok((version, tag_ids))
    }
}

async fn require_forum_channel(
    connection: &mut sqlx::SqliteConnection,
    server_id: &str,
    channel_id: &str,
) -> Result<(), ForumError> {
    let is_forum: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM channels \
         WHERE id=? AND server_id=? AND channel_type='forum')",
    )
    .bind(channel_id)
    .bind(server_id)
    .fetch_one(connection)
    .await?;
    if is_forum {
        Ok(())
    } else {
        Err(ForumError::Validation("forum tags require a forum channel"))
    }
}

async fn insert_audit(
    connection: &mut sqlx::SqliteConnection,
    server_id: &str,
    actor: &Actor,
    action_type: &str,
    target_type: &str,
    target_id: &str,
    changes: Option<&str>,
) -> Result<(), ForumError> {
    let id = Uuid::new_v4().to_string();
    crate::db::queries::audit_log::create_entry_in(
        connection,
        &CreateAuditLogParams {
            id: &id,
            server_id,
            actor_id: actor.user_id().as_str(),
            action_type,
            target_type: Some(target_type),
            target_id: Some(target_id),
            reason: None,
            changes,
        },
    )
    .await?;
    Ok(())
}

fn validate_tag(name: &str, emoji: Option<&str>, position: Option<i32>) -> Result<(), ForumError> {
    if name.trim().is_empty() || name.len() > 100 {
        return Err(ForumError::Validation(
            "forum tag name must contain 1 to 100 bytes",
        ));
    }
    if emoji.is_some_and(|value| value.is_empty() || value.len() > 100) {
        return Err(ForumError::Validation(
            "forum tag emoji must contain 1 to 100 bytes",
        ));
    }
    if position.is_some_and(|value| !(0..20).contains(&value)) {
        return Err(ForumError::Validation(
            "forum tag position must be between 0 and 19",
        ));
    }
    Ok(())
}

fn row_to_info(row: ForumTagRow) -> ForumTagInfo {
    ForumTagInfo {
        id: row.id,
        name: row.name,
        emoji: row.emoji,
        moderated: row.moderated != 0,
        position: row.position,
    }
}
