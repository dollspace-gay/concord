use super::{
    Actor, CreateAutomodRule, ModerationError, ModerationService, Permissions, ServerId,
    UpdateAutomodRule, Uuid, validate_automod_action, validate_automod_config,
    validate_automod_rule,
};

impl ModerationService {
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
