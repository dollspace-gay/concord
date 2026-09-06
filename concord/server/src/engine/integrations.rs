use std::fmt;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::authority::{Actor, AuthService, CredentialId, UserId};
use crate::db::models::{CreateAuditLogParams, WebhookRow};
use crate::engine::authorization::AuthorizationService;
use crate::engine::permissions::Permissions;
use crate::engine::write_admission::WriteAdmission;

#[derive(Debug)]
pub enum IntegrationError {
    Unavailable,
    Forbidden,
    DependencyUnavailable,
    InvalidInput(&'static str),
    Database(sqlx::Error),
    Authentication(crate::auth::authority::AuthError),
}

impl fmt::Display for IntegrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("resource unavailable"),
            Self::Forbidden => formatter.write_str("FORBIDDEN: integration operation denied"),
            Self::DependencyUnavailable => formatter
                .write_str("DEPENDENCY_UNAVAILABLE: integration write admission unavailable"),
            Self::InvalidInput(message) => write!(formatter, "INVALID_INPUT: {message}"),
            Self::Database(_) => {
                formatter.write_str("DEPENDENCY_UNAVAILABLE: integration database unavailable")
            }
            Self::Authentication(_) => {
                formatter.write_str("AUTHENTICATION_REQUIRED: credential is no longer valid")
            }
        }
    }
}

impl std::error::Error for IntegrationError {}

impl From<sqlx::Error> for IntegrationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<crate::auth::authority::AuthError> for IntegrationError {
    fn from(error: crate::auth::authority::AuthError) -> Self {
        match error {
            crate::auth::authority::AuthError::VerificationBusy
            | crate::auth::authority::AuthError::HashWorker(_) => Self::DependencyUnavailable,
            crate::auth::authority::AuthError::Database(error) => Self::Database(error),
            error => Self::Authentication(error),
        }
    }
}

impl From<crate::engine::authorization::AuthorizationError> for IntegrationError {
    fn from(error: crate::engine::authorization::AuthorizationError) -> Self {
        match error {
            crate::engine::authorization::AuthorizationError::Unavailable => Self::Forbidden,
            crate::engine::authorization::AuthorizationError::Database(error) => {
                Self::Database(error)
            }
            crate::engine::authorization::AuthorizationError::Authentication(error) => {
                Self::Authentication(error)
            }
        }
    }
}

impl From<crate::engine::write_admission::WriteAdmissionError> for IntegrationError {
    fn from(error: crate::engine::write_admission::WriteAdmissionError) -> Self {
        match error {
            crate::engine::write_admission::WriteAdmissionError::Unavailable => {
                Self::DependencyUnavailable
            }
            crate::engine::write_admission::WriteAdmissionError::Database(error) => {
                Self::Database(error)
            }
        }
    }
}

pub struct CreateWebhook<'a> {
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub name: &'a str,
    pub webhook_type: &'a str,
    pub url: Option<&'a str>,
}

pub struct CreatedWebhook {
    pub row: WebhookRow,
    pub one_time_secret: String,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct WebhookDeliveryStatus {
    pub delivery_id: String,
    pub event_type: String,
    pub event_version: i64,
    pub state: String,
    pub attempt_count: i64,
    pub last_status: Option<i64>,
    pub safe_error_code: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

#[derive(Clone)]
pub struct IntegrationService {
    pool: SqlitePool,
    auth: AuthService,
    authorization: AuthorizationService,
    writes: WriteAdmission,
    vault: std::sync::Arc<crate::secrets::SecretVault>,
}

impl IntegrationService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: WriteAdmission,
        vault: std::sync::Arc<crate::secrets::SecretVault>,
    ) -> Self {
        Self {
            authorization: AuthorizationService::new(pool.clone()),
            pool,
            auth,
            writes,
            vault,
        }
    }

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

    async fn pool_connection(
        &self,
    ) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>, IntegrationError> {
        // A read transaction keeps authorization and returned status on one snapshot.
        Ok(self.pool.begin().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::engine::write_admission::WriteAdmission;

    async fn fixture() -> (SqlitePool, AuthService, Actor, IntegrationService) {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        for (id, username) in [("owner", "owner"), ("other", "other")] {
            sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
                .bind(id)
                .bind(username)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query(
            "INSERT INTO servers(id,name,owner_id) VALUES \
             ('server','Server','owner'),('other-server','Other','other')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('server','owner','owner'),('other-server','other','owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES \
             ('channel','server','#channel'),('other-channel','other-server','#other')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "integration-secret".into(), 2);
        let (_, actor) = auth.issue_web_session("owner").await.unwrap();
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([17_u8; 32])).unwrap();
        let vault =
            std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
        let service = IntegrationService::new(
            pool.clone(),
            auth.clone(),
            WriteAdmission::new(pool.clone()),
            vault,
        );
        (pool, auth, actor, service)
    }

    fn incoming<'a>() -> CreateWebhook<'a> {
        CreateWebhook {
            server_id: "server",
            channel_id: "channel",
            name: "Hook",
            webhook_type: "incoming",
            url: None,
        }
    }

    #[tokio::test]
    async fn every_create_row_and_audit_failure_rolls_back_identity_grant_and_credential() {
        for table in [
            "users",
            "bot_ownership",
            "server_members",
            "bot_installations",
            "auth_credentials",
            "bot_tokens",
            "webhooks",
            "audit_log",
        ] {
            let (pool, _auth, actor, service) = fixture().await;
            let trigger = format!(
                "CREATE TRIGGER reject_{table} BEFORE INSERT ON {table} \
                 WHEN {} BEGIN SELECT RAISE(FAIL,'injected'); END",
                if table == "users" {
                    "NEW.is_bot=1"
                } else if table == "server_members" {
                    "NEW.user_id LIKE 'webhook:%'"
                } else if table == "auth_credentials" {
                    "NEW.kind='bot_token'"
                } else if table == "audit_log" {
                    "NEW.action_type='webhook_create'"
                } else {
                    "1"
                }
            );
            // Test trigger identifiers and predicates come only from the fixed literals above.
            sqlx::query(sqlx::AssertSqlSafe(trigger))
                .execute(&pool)
                .await
                .unwrap();
            assert!(service.create_webhook(&actor, incoming()).await.is_err());
            for query in [
                "SELECT COUNT(*) FROM users WHERE id LIKE 'webhook:%'",
                "SELECT COUNT(*) FROM bot_installations",
                "SELECT COUNT(*) FROM auth_credentials WHERE kind='bot_token'",
                "SELECT COUNT(*) FROM bot_tokens",
                "SELECT COUNT(*) FROM webhooks",
                "SELECT COUNT(*) FROM audit_log WHERE action_type='webhook_create'",
            ] {
                let count: i64 = sqlx::query_scalar(query).fetch_one(&pool).await.unwrap();
                assert_eq!(count, 0, "{table} fault left state for {query}");
            }
        }
    }

    #[tokio::test]
    async fn failed_delete_restores_webhook_principal_and_usable_credential() {
        let (pool, auth, actor, service) = fixture().await;
        let created = service.create_webhook(&actor, incoming()).await.unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_webhook_delete_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='webhook_delete' BEGIN SELECT RAISE(FAIL,'injected'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            service
                .delete_webhook(&actor, &created.row.id)
                .await
                .is_err()
        );
        assert!(
            crate::db::queries::webhooks::get_webhook(&pool, &created.row.id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND is_bot=1)"
            )
            .bind(created.row.principal_user_id.as_deref().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap()
        );
        auth.authenticate_bot(&created.one_time_secret)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn committed_delete_removes_all_state_and_cancels_live_credential() {
        let (pool, auth, actor, service) = fixture().await;
        let created = service.create_webhook(&actor, incoming()).await.unwrap();
        let bot_actor = auth
            .authenticate_bot(&created.one_time_secret)
            .await
            .unwrap();
        let lease = auth.register_live(&bot_actor).await.unwrap();
        service
            .delete_webhook(&actor, &created.row.id)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), lease.cancelled())
            .await
            .unwrap();
        assert!(
            auth.authenticate_bot(&created.one_time_secret)
                .await
                .is_err()
        );
        for query in [
            "SELECT COUNT(*) FROM webhooks",
            "SELECT COUNT(*) FROM users WHERE id LIKE 'webhook:%'",
            "SELECT COUNT(*) FROM bot_installations",
            "SELECT COUNT(*) FROM auth_credentials WHERE kind='bot_token'",
            "SELECT COUNT(*) FROM bot_tokens",
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(query)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0,
                "delete left state for {query}"
            );
        }
    }

    #[tokio::test]
    async fn outgoing_secret_is_recoverable_only_from_vault_and_controls_are_transactional() {
        let (pool, _auth, actor, service) = fixture().await;
        let created = service
            .create_webhook(
                &actor,
                CreateWebhook {
                    server_id: "server",
                    channel_id: "channel",
                    name: "Outgoing",
                    webhook_type: "outgoing",
                    url: Some("https://example.com/hook"),
                },
            )
            .await
            .unwrap();
        assert!(!created.row.token.contains(&created.one_time_secret));
        assert!(
            !created
                .row
                .signing_ciphertext
                .as_deref()
                .unwrap()
                .contains(&created.one_time_secret)
        );
        let delivery_id = service
            .enqueue_test_delivery(&actor, &created.row.id)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE webhook_deliveries SET state='failed',attempt_count=8, \
             safe_error_code='test_failure' WHERE delivery_id=?",
        )
        .bind(&delivery_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE external_jobs SET state='failed',attempt_count=8,safe_error_code='test_failure' \
             WHERE resource_id=?",
        )
        .bind(&delivery_id)
        .execute(&pool)
        .await
        .unwrap();
        let failed = service
            .list_deliveries(&actor, &created.row.id, 10)
            .await
            .unwrap();
        assert_eq!(failed[0].state, "failed");
        service.retry_delivery(&actor, &delivery_id).await.unwrap();
        let retried = service
            .list_deliveries(&actor, &created.row.id, 10)
            .await
            .unwrap();
        assert_eq!(retried[0].state, "pending");
        assert_eq!(retried[0].attempt_count, 0);
        let job: (String, i64, Option<String>) = sqlx::query_as(
            "SELECT state,attempt_count,safe_error_code FROM external_jobs WHERE resource_id=?",
        )
        .bind(&delivery_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(job, ("pending".into(), 0, None));
    }

    #[tokio::test]
    async fn moving_outgoing_webhook_versions_grant_and_cancels_old_scope_queue() {
        let (pool, _auth, actor, service) = fixture().await;
        sqlx::query(
            "INSERT INTO channels(id,server_id,name) VALUES('sibling','server','#sibling')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let created = service
            .create_webhook(
                &actor,
                CreateWebhook {
                    server_id: "server",
                    channel_id: "channel",
                    name: "Outgoing",
                    webhook_type: "outgoing",
                    url: Some("https://example.com/hook"),
                },
            )
            .await
            .unwrap();
        let delivery_id = service
            .enqueue_test_delivery(&actor, &created.row.id)
            .await
            .unwrap();
        let updated = service
            .update_webhook(&actor, &created.row.id, "Moved", None, "sibling")
            .await
            .unwrap();
        assert_eq!(updated.channel_id, "sibling");
        assert_eq!(updated.grant_version, created.row.grant_version + 1);
        let states: (String, String, Option<String>) = sqlx::query_as(
            "SELECT d.state,j.state,d.safe_error_code FROM webhook_deliveries d \
             JOIN external_jobs j ON j.id=d.external_job_id WHERE d.delivery_id=?",
        )
        .bind(delivery_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            states,
            (
                "cancelled".into(),
                "cancelled".into(),
                Some("webhook_scope_changed".into())
            )
        );
    }

    #[tokio::test]
    async fn lifecycle_revalidates_actor_and_channel_server_inside_each_transaction() {
        let (pool, _auth, actor, service) = fixture().await;
        let created = service.create_webhook(&actor, incoming()).await.unwrap();
        assert!(
            service
                .update_webhook(&actor, &created.row.id, "Moved", None, "other-channel")
                .await
                .is_err()
        );
        sqlx::query("UPDATE servers SET owner_id='other' WHERE id='server'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM server_members WHERE server_id='server' AND user_id='owner'")
            .execute(&pool)
            .await
            .unwrap();
        let denied = service
            .update_webhook(&actor, &created.row.id, "Renamed", None, "channel")
            .await
            .unwrap_err();
        assert!(denied.to_string().starts_with("FORBIDDEN:"));
        assert!(
            service
                .delete_webhook(&actor, &created.row.id)
                .await
                .is_err()
        );
        let unchanged = crate::db::queries::webhooks::get_webhook(&pool, &created.row.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.name, "Hook");
    }

    #[tokio::test]
    async fn write_admission_timeout_keeps_retryable_dependency_error_code() {
        let (pool, _auth, actor, service) = fixture().await;
        let created = service.create_webhook(&actor, incoming()).await.unwrap();
        let mut blocker = pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        let error = service
            .update_webhook(&actor, &created.row.id, "Blocked", None, "channel")
            .await
            .unwrap_err();
        assert!(error.to_string().starts_with("DEPENDENCY_UNAVAILABLE:"));
        sqlx::query("ROLLBACK")
            .execute(&mut *blocker)
            .await
            .unwrap();
    }
}
