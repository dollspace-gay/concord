use sqlx::SqlitePool;

use crate::db::models::InviteRow;

#[allow(clippy::too_many_arguments)]
pub async fn create_invite(
    pool: &SqlitePool,
    id: &str,
    server_id: &str,
    code: &str,
    created_by: &str,
    max_uses: Option<i32>,
    expires_at: Option<&str>,
    channel_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO invites (id, server_id, code, created_by, max_uses, expires_at, channel_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(server_id)
    .bind(code)
    .bind(created_by)
    .bind(max_uses)
    .bind(expires_at)
    .bind(channel_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_invite_by_code(
    pool: &SqlitePool,
    code: &str,
) -> Result<Option<InviteRow>, sqlx::Error> {
    sqlx::query_as::<_, InviteRow>("SELECT * FROM invites WHERE code = ?")
        .bind(code)
        .fetch_optional(pool)
        .await
}

pub async fn list_server_invites(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<InviteRow>, sqlx::Error> {
    sqlx::query_as::<_, InviteRow>(
        "SELECT * FROM invites WHERE server_id = ? ORDER BY created_at DESC",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn increment_use_count(pool: &SqlitePool, invite_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE invites SET use_count = use_count + 1 WHERE id = ?")
        .bind(invite_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_invite(pool: &SqlitePool, invite_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM invites WHERE id = ?")
        .bind(invite_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_expired_invites(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM invites WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
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

    #[tokio::test]
    async fn test_create_and_get_invite() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_invite(&pool, "inv1", "s1", "ABC123", "u1", None, None, None)
            .await
            .unwrap();

        let inv = get_invite_by_code(&pool, "ABC123").await.unwrap();
        assert!(inv.is_some());
        let i = inv.unwrap();
        assert_eq!(i.code, "ABC123");
        assert_eq!(i.server_id, "s1");
        assert_eq!(i.use_count, 0);
        assert!(i.max_uses.is_none());
        assert!(i.expires_at.is_none());
    }

    #[tokio::test]
    async fn test_invite_with_max_uses() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_invite(&pool, "inv1", "s1", "LIM5", "u1", Some(5), None, None)
            .await
            .unwrap();

        let inv = get_invite_by_code(&pool, "LIM5").await.unwrap().unwrap();
        assert_eq!(inv.max_uses, Some(5));
    }

    #[tokio::test]
    async fn test_list_server_invites() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_invite(&pool, "inv1", "s1", "CODE1", "u1", None, None, None)
            .await
            .unwrap();
        create_invite(&pool, "inv2", "s1", "CODE2", "u1", None, None, None)
            .await
            .unwrap();

        let invites = list_server_invites(&pool, "s1").await.unwrap();
        assert_eq!(invites.len(), 2);
    }

    #[tokio::test]
    async fn test_increment_use_count() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_invite(&pool, "inv1", "s1", "CODE1", "u1", None, None, None)
            .await
            .unwrap();

        increment_use_count(&pool, "inv1").await.unwrap();
        increment_use_count(&pool, "inv1").await.unwrap();

        let inv = get_invite_by_code(&pool, "CODE1").await.unwrap().unwrap();
        assert_eq!(inv.use_count, 2);
    }

    #[tokio::test]
    async fn test_delete_invite() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_invite(&pool, "inv1", "s1", "CODE1", "u1", None, None, None)
            .await
            .unwrap();

        delete_invite(&pool, "inv1").await.unwrap();

        let inv = get_invite_by_code(&pool, "CODE1").await.unwrap();
        assert!(inv.is_none());
    }

    #[tokio::test]
    async fn test_delete_expired_invites() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        // Create an already-expired invite
        create_invite(
            &pool,
            "inv1",
            "s1",
            "EXPIRED",
            "u1",
            None,
            Some("2020-01-01T00:00:00Z"),
            None,
        )
        .await
        .unwrap();
        // Create a non-expired invite
        create_invite(
            &pool,
            "inv2",
            "s1",
            "VALID",
            "u1",
            None,
            Some("2099-01-01T00:00:00Z"),
            None,
        )
        .await
        .unwrap();

        let deleted = delete_expired_invites(&pool).await.unwrap();
        assert_eq!(deleted, 1);

        let valid = get_invite_by_code(&pool, "VALID").await.unwrap();
        assert!(valid.is_some());
        let expired = get_invite_by_code(&pool, "EXPIRED").await.unwrap();
        assert!(expired.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_invite() {
        let pool = setup_db().await;
        let inv = get_invite_by_code(&pool, "NOSUCH").await.unwrap();
        assert!(inv.is_none());
    }
}
