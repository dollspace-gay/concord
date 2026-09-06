use super::{
    Actor, CreateAuditLogParams, IntegrationError, IntegrationService, Permissions, Uuid,
    WebhookDeliveryStatus, WebhookRow,
};

impl IntegrationService {
    pub async fn list_deliveries(
        &self,
        actor: &Actor,
        webhook_id: &str,
        limit: i64,
    ) -> Result<Vec<WebhookDeliveryStatus>, IntegrationError> {
        let mut transaction = self.pool_connection().await?;
        let server_id: String = sqlx::query_scalar("SELECT server_id FROM webhooks WHERE id=?")
            .bind(webhook_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(IntegrationError::Unavailable)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(IntegrationError::from)?;
        let rows = sqlx::query_as::<_, WebhookDeliveryStatus>(
            "SELECT delivery_id,event_type,event_version,state,attempt_count,last_status, \
                    safe_error_code,created_at,delivered_at \
             FROM webhook_deliveries WHERE webhook_id=? ORDER BY created_at DESC,id DESC LIMIT ?",
        )
        .bind(webhook_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows)
    }

    pub async fn enqueue_test_delivery(
        &self,
        actor: &Actor,
        webhook_id: &str,
    ) -> Result<String, IntegrationError> {
        let (_permit, mut transaction) =
            self.writes.begin().await.map_err(IntegrationError::from)?;
        let webhook: WebhookRow = sqlx::query_as("SELECT * FROM webhooks WHERE id=?")
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
        if webhook.webhook_type != "outgoing"
            || webhook.credential_state != "active"
            || webhook.revoked_at.is_some()
        {
            return Err(IntegrationError::Unavailable);
        }
        let delivery_id = Uuid::new_v4().to_string();
        let job_id = Uuid::new_v4().to_string();
        let payload = serde_json::json!({
            "delivery_id": delivery_id,
            "event_type": "webhook_test",
            "event_version": 1,
            "server_id": webhook.server_id,
            "channel_id": webhook.channel_id,
            "data": {"requested_by": actor.user_id().as_str()},
        })
        .to_string();
        sqlx::query("INSERT INTO external_jobs(id,deduplication_key,operation_type,resource_id,resource_version,destination_grant,payload_json) VALUES(?,?,'webhook_delivery',?,1,?,?)")
            .bind(&job_id)
            .bind(format!("webhook:{webhook_id}:test:{delivery_id}"))
            .bind(&delivery_id)
            .bind(format!("webhook:{webhook_id}:{}", webhook.grant_version))
            .bind(&payload)
            .execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO webhook_deliveries(id,webhook_id,external_job_id,delivery_id,event_type,event_version,payload_json) VALUES(?,?,?,?, 'webhook_test',1,?)")
            .bind(Uuid::new_v4().to_string()).bind(webhook_id).bind(&job_id)
            .bind(&delivery_id).bind(&payload).execute(&mut *transaction).await?;
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &CreateAuditLogParams {
                id: &Uuid::new_v4().to_string(),
                server_id: &webhook.server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "webhook_test",
                target_type: Some("webhook"),
                target_id: Some(webhook_id),
                reason: None,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(delivery_id)
    }

    pub async fn retry_delivery(
        &self,
        actor: &Actor,
        delivery_id: &str,
    ) -> Result<(), IntegrationError> {
        let (_permit, mut transaction) =
            self.writes.begin().await.map_err(IntegrationError::from)?;
        let row: (String, String, String, i64) = sqlx::query_as(
            "SELECT w.server_id,w.id,d.external_job_id,w.grant_version \
             FROM webhook_deliveries d JOIN webhooks w ON w.id=d.webhook_id \
             WHERE d.delivery_id=? AND d.state='failed' AND w.revoked_at IS NULL",
        )
        .bind(delivery_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IntegrationError::Unavailable)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                &row.0,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(IntegrationError::from)?;
        let changed = sqlx::query(
            "UPDATE external_jobs SET state='pending',attempt_count=0,next_attempt_at=datetime('now'), \
             lease_owner=NULL,lease_token=NULL,lease_until=NULL,safe_error_code=NULL, \
             destination_grant=?,updated_at=datetime('now') WHERE id=? AND state='failed'",
        )
        .bind(format!("webhook:{}:{}", row.1, row.3))
        .bind(&row.2)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(IntegrationError::Unavailable);
        }
        sqlx::query("UPDATE webhook_deliveries SET state='pending',attempt_count=0,last_status=NULL,safe_error_code=NULL WHERE delivery_id=?")
            .bind(delivery_id).execute(&mut *transaction).await?;
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &CreateAuditLogParams {
                id: &Uuid::new_v4().to_string(),
                server_id: &row.0,
                actor_id: actor.user_id().as_str(),
                action_type: "webhook_delivery_retry",
                target_type: Some("webhook"),
                target_id: Some(&row.1),
                reason: None,
                changes: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn pool_connection(
        &self,
    ) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>, IntegrationError> {
        // A read transaction keeps authorization and returned status on one snapshot.
        Ok(self.pool.begin().await?)
    }
}
