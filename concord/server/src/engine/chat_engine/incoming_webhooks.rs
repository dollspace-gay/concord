use super::{ChatEngine, ConnectionId};

impl ChatEngine {
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
    ) -> Result<crate::engine::messaging::CommandReceipt, String> {
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
                crate::engine::messaging::SendMessageCommand {
                    request_id: idempotency_key,
                    client_message_id: idempotency_key,
                    operation_generation: None,
                    conversation_id: Some(&conversation_id),
                    server_id: &wh.server_id,
                    channel: "",
                    content,
                    content_format: crate::engine::messaging::ContentFormat::Markdown,
                    reply_to_id: None,
                    attachment_ids: &attachments,
                    mentions: &mentions,
                },
            )
            .await
            .map_err(|error| error.to_string())
    }
    /// Helper: get user_id for a session.
    pub(super) fn get_user_id(&self, session_id: ConnectionId) -> Result<String, String> {
        let session = self.sessions.get(&session_id).ok_or("Session not found")?;
        session
            .user_id
            .clone()
            .ok_or_else(|| "AUTH_REQUIRED".into())
    }
    /// Helper: resolve channel name from a channel_id by looking it up in self.channels.
    pub(super) fn resolve_channel_name_from_id(&self, channel_id: &str) -> Result<String, String> {
        self.channels
            .get(channel_id)
            .map(|ch| ch.name.clone())
            .ok_or_else(|| format!("Channel ID {channel_id} not found"))
    }
}
