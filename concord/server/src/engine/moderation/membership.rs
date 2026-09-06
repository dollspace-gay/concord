use super::{
    Actor, ChannelId, ModerationError, ModerationService, Permissions, ServerId, Utc, Uuid,
};

impl ModerationService {
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
}
