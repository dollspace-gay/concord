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
    let vault = std::sync::Arc::new(crate::secrets::SecretVault::load(key_file.path()).unwrap());
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
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND is_bot=1)")
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
    sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('sibling','server','#sibling')")
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
