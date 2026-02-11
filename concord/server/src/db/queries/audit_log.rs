use sqlx::SqlitePool;

use crate::db::models::{AuditLogRow, CreateAuditLogParams};

pub async fn create_entry(
    pool: &SqlitePool,
    params: &CreateAuditLogParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_log (id, server_id, actor_id, action_type, target_type, target_id, reason, changes) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(params.id)
    .bind(params.server_id)
    .bind(params.actor_id)
    .bind(params.action_type)
    .bind(params.target_type)
    .bind(params.target_id)
    .bind(params.reason)
    .bind(params.changes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_entries(
    pool: &SqlitePool,
    server_id: &str,
    action_type: Option<&str>,
    limit: i64,
    before: Option<&str>,
) -> Result<Vec<AuditLogRow>, sqlx::Error> {
    match (action_type, before) {
        (Some(at), Some(b)) => {
            sqlx::query_as::<_, AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id = ? AND action_type = ? AND created_at < ? ORDER BY created_at DESC LIMIT ?",
            )
            .bind(server_id)
            .bind(at)
            .bind(b)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (Some(at), None) => {
            sqlx::query_as::<_, AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id = ? AND action_type = ? ORDER BY created_at DESC LIMIT ?",
            )
            .bind(server_id)
            .bind(at)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (None, Some(b)) => {
            sqlx::query_as::<_, AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id = ? AND created_at < ? ORDER BY created_at DESC LIMIT ?",
            )
            .bind(server_id)
            .bind(b)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, AuditLogRow>(
                "SELECT * FROM audit_log WHERE server_id = ? ORDER BY created_at DESC LIMIT ?",
            )
            .bind(server_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::servers;
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn setup_server(pool: &SqlitePool) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: "u1",
                username: "alice",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u1",
                provider: "github",
                provider_id: "gh-u1",
            },
        )
        .await
        .unwrap();
        servers::create_server(pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
    }

    fn entry_params<'a>(id: &'a str, action: &'a str) -> CreateAuditLogParams<'a> {
        CreateAuditLogParams {
            id,
            server_id: "s1",
            actor_id: "u1",
            action_type: action,
            target_type: Some("user"),
            target_id: Some("u2"),
            reason: Some("Test reason"),
            changes: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_list_entries() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_entry(&pool, &entry_params("al1", "ban"))
            .await
            .unwrap();
        create_entry(&pool, &entry_params("al2", "kick"))
            .await
            .unwrap();

        let entries = list_entries(&pool, "s1", None, 50, None).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_filter_by_action_type() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_entry(&pool, &entry_params("al1", "ban"))
            .await
            .unwrap();
        create_entry(&pool, &entry_params("al2", "kick"))
            .await
            .unwrap();
        create_entry(&pool, &entry_params("al3", "ban"))
            .await
            .unwrap();

        let bans = list_entries(&pool, "s1", Some("ban"), 50, None)
            .await
            .unwrap();
        assert_eq!(bans.len(), 2);

        let kicks = list_entries(&pool, "s1", Some("kick"), 50, None)
            .await
            .unwrap();
        assert_eq!(kicks.len(), 1);
    }

    #[tokio::test]
    async fn test_pagination_with_limit() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        for i in 0..5 {
            create_entry(&pool, &entry_params(&format!("al{i}"), "action"))
                .await
                .unwrap();
        }

        let entries = list_entries(&pool, "s1", None, 2, None).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_pagination_with_before() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_entry(&pool, &entry_params("al1", "ban"))
            .await
            .unwrap();

        // Use a future date to get all entries
        let entries = list_entries(&pool, "s1", None, 50, Some("2099-01-01T00:00:00Z"))
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);

        // Use a past date to get none
        let entries = list_entries(&pool, "s1", None, 50, Some("2000-01-01T00:00:00Z"))
            .await
            .unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_entry_with_changes() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_entry(
            &pool,
            &CreateAuditLogParams {
                id: "al1",
                server_id: "s1",
                actor_id: "u1",
                action_type: "role_update",
                target_type: Some("role"),
                target_id: Some("r1"),
                reason: None,
                changes: Some("{\"name\":{\"old\":\"Mod\",\"new\":\"Admin\"}}"),
            },
        )
        .await
        .unwrap();

        let entries = list_entries(&pool, "s1", None, 50, None).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].changes.is_some());
    }

    #[tokio::test]
    async fn test_empty_audit_log() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        let entries = list_entries(&pool, "s1", None, 50, None).await.unwrap();
        assert!(entries.is_empty());
    }
}
