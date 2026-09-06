use super::{
    Actor, CreateAuditLogParams, CreateWebhook, CreatedWebhook, IntegrationError,
    IntegrationService, Permissions, UserId, Uuid, WebhookRow,
};

impl IntegrationService {
    pub async fn create_webhook(
        &self,
        actor: &Actor,
        input: CreateWebhook<'_>,
    ) -> Result<CreatedWebhook, IntegrationError> {
        if !matches!(input.webhook_type, "incoming" | "outgoing") {
            return Err(IntegrationError::InvalidInput(
                "webhook_type must be 'incoming' or 'outgoing'",
            ));
        }
        if input.webhook_type == "outgoing" && input.url.is_none() {
            return Err(IntegrationError::InvalidInput(
                "outgoing webhook URL is required",
            ));
        }
        let mut preflight = self.pool.begin().await?;
        self.authorization
            .require_server_actor_in(
                &mut preflight,
                &self.auth,
                actor,
                input.server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(IntegrationError::from)?;
        let channel_server: Option<String> =
            sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                .bind(input.channel_id)
                .fetch_optional(&mut *preflight)
                .await?;
        if channel_server.as_deref() != Some(input.server_id) {
            return Err(IntegrationError::Unavailable);
        }
        preflight.commit().await?;
        let webhook_id = Uuid::new_v4().to_string();
        let principal_id = format!("webhook:{webhook_id}");
        let required_scope = format!("webhook:channel:{}", input.channel_id);
        let prepared = if input.webhook_type == "incoming" {
            Some(
                self.auth
                    .prepare_bot_token("Incoming webhook", &format!("bot {required_scope}"))
                    .await?,
            )
        } else {
            None
        };
        let outgoing_secret = (input.webhook_type == "outgoing")
            .then(|| format!("{}.{}", webhook_id, Uuid::new_v4()));
        let outgoing_hash = match outgoing_secret.as_ref() {
            Some(secret) => Some(self.auth.hash_secret(secret.clone()).await?),
            None => None,
        };
        let signing_context = format!("webhook:{webhook_id}:signing");
        let signing_ciphertext = outgoing_secret
            .as_deref()
            .map(|secret| self.vault.encrypt(&signing_context, secret.as_bytes()))
            .transpose()
            .map_err(|_| IntegrationError::Unavailable)?;

        let (_permit, mut transaction) =
            self.writes.begin().await.map_err(IntegrationError::from)?;
        self.authorization
            .require_server_actor_in(
                &mut transaction,
                &self.auth,
                actor,
                input.server_id,
                Permissions::MANAGE_SERVER,
            )
            .await
            .map_err(IntegrationError::from)?;
        let channel_server: Option<String> =
            sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                .bind(input.channel_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if channel_server.as_deref() != Some(input.server_id) {
            return Err(IntegrationError::Unavailable);
        }

        let (credential_id, stored_token, principal_user_id, credential_state) =
            if let Some(prepared) = prepared.as_ref() {
                sqlx::query("INSERT INTO users(id,username,is_bot) VALUES(?,?,1)")
                    .bind(&principal_id)
                    .bind(format!("webhook-{}", &webhook_id[..8]))
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query(
                    "INSERT INTO bot_ownership(bot_user_id,owner_user_id,repair_required) \
                     VALUES(?,?,0)",
                )
                .bind(&principal_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,'member')",
                )
                .bind(input.server_id)
                .bind(&principal_id)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT INTO bot_installations( \
                        id,bot_user_id,server_id,installed_by,granted_scopes,state \
                     ) VALUES(?,?,?,?,?,'active')",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(&principal_id)
                .bind(input.server_id)
                .bind(actor.user_id().as_str())
                .bind(&required_scope)
                .execute(&mut *transaction)
                .await?;
                let principal = UserId::from_stored(principal_id.clone())?;
                self.auth
                    .insert_prepared_bot_in(&mut transaction, &principal, prepared)
                    .await?;
                (
                    Some(prepared.credential_id().as_str()),
                    prepared.credential_id().as_str(),
                    Some(principal_id.as_str()),
                    "active",
                )
            } else {
                (None, outgoing_hash.as_deref().unwrap_or(""), None, "active")
            };
        sqlx::query(
            "INSERT INTO webhooks( \
                id,server_id,channel_id,name,webhook_type,token,url,created_by, \
                credential_id,principal_user_id,credential_state,signing_key_id,signing_ciphertext \
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&webhook_id)
        .bind(input.server_id)
        .bind(input.channel_id)
        .bind(input.name)
        .bind(input.webhook_type)
        .bind(stored_token)
        .bind(input.url)
        .bind(actor.user_id().as_str())
        .bind(credential_id)
        .bind(principal_user_id)
        .bind(credential_state)
        .bind(outgoing_secret.as_ref().map(|_| self.vault.key_id()))
        .bind(signing_ciphertext)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE id=?")
            .bind(&webhook_id)
            .fetch_one(&mut *transaction)
            .await?;
        let changes = serde_json::json!({
            "channel_id": input.channel_id,
            "webhook_type": input.webhook_type,
        })
        .to_string();
        crate::db::queries::audit_log::create_entry_in(
            &mut transaction,
            &CreateAuditLogParams {
                id: &Uuid::new_v4().to_string(),
                server_id: input.server_id,
                actor_id: actor.user_id().as_str(),
                action_type: "webhook_create",
                target_type: Some("webhook"),
                target_id: Some(&webhook_id),
                reason: None,
                changes: Some(&changes),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(CreatedWebhook {
            row,
            one_time_secret: prepared
                .map(|prepared| prepared.secret().to_owned())
                .or(outgoing_secret)
                .unwrap_or_default(),
        })
    }
}
