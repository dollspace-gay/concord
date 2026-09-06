use super::{
    Actor, ChannelId, HashSet, ModerationError, ModerationService, Permissions, Row, ServerId,
    SqliteConnection, Uuid,
};

impl ModerationService {
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

    pub(super) async fn audit_channel_change_in(
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
            crate::engine::messaging::tombstone_moderated_message_in(
                &mut transaction,
                &generation,
                &message_id,
                actor.user_id().as_str(),
            )
            .await
            .map_err(|error| match error {
                crate::engine::messaging::MessagingError::Internal(error) => {
                    ModerationError::Database(error)
                }
                crate::engine::messaging::MessagingError::DependencyUnavailable => {
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
}
