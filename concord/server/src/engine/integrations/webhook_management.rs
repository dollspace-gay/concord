use super::{
    Actor, CreateAuditLogParams, CredentialId, IntegrationError, IntegrationService, Permissions,
    Uuid, WebhookRow,
};

impl IntegrationService {
    pub async fn update_webhook(
        &self,
        actor: &Actor,
        webhook_id: &str,
        name: &str,
        avatar_url: Option<&str>,
        channel_id: &str,
    ) -> Result<WebhookRow, IntegrationError> {
        let (_permit, mut transaction) =
            self.writes.begin().await.map_err(IntegrationError::from)?;
        let webhook = sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE id=?")
            .bind(webhook_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(IntegrationError::Unavailable)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &webhook.server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(IntegrationError::from)?;
        let channel_server: Option<String> =
            sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                .bind(channel_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if channel_server.as_deref() != Some(webhook.server_id.as_str()) {
            return Err(IntegrationError::Unavailable);
        }
        if webhook.webhook_type == "incoming" && webhook.channel_id != channel_id {
            return Err(IntegrationError::InvalidInput(
                "incoming webhook channel is fixed; create a new webhook instead",
            ));
        }
        let channel_changed =
            webhook.webhook_type == "outgoing" && webhook.channel_id != channel_id;
        sqlx::query(
            "UPDATE webhooks SET name=?,avatar_url=?,channel_id=?, \
             grant_version=grant_version+? WHERE id=?",
        )
        .bind(name)
        .bind(avatar_url)
        .bind(channel_id)
        .bind(i64::from(channel_changed))
        .bind(webhook_id)
        .execute(&mut *transaction)
        .await?;
        if channel_changed {
            sqlx::query(
                "UPDATE external_jobs SET state='cancelled',lease_owner=NULL,lease_token=NULL, \
                 lease_until=NULL,updated_at=datetime('now') \
                 WHERE id IN (SELECT external_job_id FROM webhook_deliveries WHERE webhook_id=?) \
                   AND state IN ('pending','leased')",
            )
            .bind(webhook_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE webhook_deliveries SET state='cancelled',safe_error_code='webhook_scope_changed' \
                 WHERE webhook_id=? AND state IN ('pending','leased')",
            )
            .bind(webhook_id)
            .execute(&mut *transaction)
            .await?;
        }
        let updated = sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE id=?")
            .bind(webhook_id)
            .fetch_one(&mut *transaction)
            .await?;
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &CreateAuditLogParams {
                id: &Uuid::new_v4().to_string(),
                server_id: &webhook.server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "webhook_update",
                target_type: Some("webhook"),
                target_id: Some(webhook_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(updated)
    }

    pub async fn delete_webhook(
        &self,
        actor: &Actor,
        webhook_id: &str,
    ) -> Result<String, IntegrationError> {
        let (_permit, mut transaction) =
            self.writes.begin().await.map_err(IntegrationError::from)?;
        let webhook = sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE id=?")
            .bind(webhook_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(IntegrationError::Unavailable)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &webhook.server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(IntegrationError::from)?;
        let revoked_credential = webhook
            .credential_id
            .as_deref()
            .map(CredentialId::from_stored)
            .transpose()?;
        if let Some(credential_id) = revoked_credential.as_ref() {
            self.auth
                .revoke_credential_in(&mut transaction, credential_id)
                .await?;
        }
        sqlx::query(
            "UPDATE external_jobs SET state='cancelled',lease_owner=NULL,lease_token=NULL, \
             lease_until=NULL,updated_at=datetime('now') \
             WHERE id IN (SELECT external_job_id FROM webhook_deliveries WHERE webhook_id=?) \
               AND state IN ('pending','leased')",
        )
        .bind(webhook_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM webhooks WHERE id=?")
            .bind(webhook_id)
            .execute(&mut *transaction)
            .await?;
        if let Some(principal_id) = webhook.principal_user_id.as_deref() {
            sqlx::query("DELETE FROM users WHERE id=? AND is_bot=1")
                .bind(principal_id)
                .execute(&mut *transaction)
                .await?;
        }
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &CreateAuditLogParams {
                id: &Uuid::new_v4().to_string(),
                server_id: &webhook.server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "webhook_delete",
                target_type: Some("webhook"),
                target_id: Some(webhook_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        if let Some(credential_id) = revoked_credential.as_ref() {
            self.auth.cancel_live_credential(credential_id);
        }
        Ok(webhook.server_id)
    }
}
