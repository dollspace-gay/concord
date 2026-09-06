use super::{ChatEngine, ChatEvent, ConnectionId, Permissions, WebhookInfo, webhook_row_to_info};

impl ChatEngine {
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
                crate::engine::integrations::CreateWebhook {
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
    ) -> Result<Vec<crate::engine::integrations::WebhookDeliveryStatus>, String> {
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
}
