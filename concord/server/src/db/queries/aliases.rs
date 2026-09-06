use sqlx::SqlitePool;

pub async fn resolve_user_alias(
    pool: &SqlitePool,
    alias: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT a.user_id FROM user_aliases a JOIN users u ON u.id=a.user_id \
         WHERE a.alias=? COLLATE NOCASE AND u.disabled_at IS NULL LIMIT 1",
    )
    .bind(alias)
    .fetch_optional(pool)
    .await
}

pub async fn resolve_server_alias(
    pool: &SqlitePool,
    alias: &str,
    user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT a.server_id FROM server_aliases a \
         JOIN server_members sm ON sm.server_id=a.server_id AND sm.user_id=? \
         WHERE a.alias=? COLLATE NOCASE \
           AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=a.server_id AND b.user_id=?) \
         LIMIT 1",
    )
    .bind(user_id)
    .bind(alias)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn resolve_channel_alias(
    pool: &SqlitePool,
    server_id: &str,
    alias: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT channel_id FROM channel_aliases \
         WHERE server_id=? AND alias=? COLLATE NOCASE LIMIT 1",
    )
    .bind(server_id)
    .bind(alias.trim_start_matches('#'))
    .fetch_optional(pool)
    .await
}

pub async fn get_default_server(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT d.server_id FROM user_default_servers d \
         JOIN server_members sm ON sm.server_id=d.server_id AND sm.user_id=d.user_id \
         WHERE d.user_id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=d.server_id AND b.user_id=d.user_id)",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn set_default_server(
    pool: &SqlitePool,
    user_id: &str,
    server_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO user_default_servers(user_id,server_id,updated_at) \
         SELECT ?,?,datetime('now') WHERE EXISTS(SELECT 1 FROM server_members WHERE user_id=? AND server_id=?) \
         ON CONFLICT(user_id) DO UPDATE SET server_id=excluded.server_id,updated_at=excluded.updated_at",
    )
    .bind(user_id)
    .bind(server_id)
    .bind(user_id)
    .bind(server_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};

    #[tokio::test]
    async fn aliases_resolve_stable_ids_and_defaults_require_membership() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('u1','Alice'),('u2','Bob')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO user_aliases(alias,user_id,alias_kind) VALUES('alice-irc','u1','nickname')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('s1','My Server','u1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('s1','u1','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_aliases(alias,server_id) VALUES('my-server','s1')")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            resolve_user_alias(&pool, "ALICE-IRC")
                .await
                .unwrap()
                .as_deref(),
            Some("u1")
        );
        assert_eq!(
            resolve_server_alias(&pool, "MY-SERVER", "u1")
                .await
                .unwrap()
                .as_deref(),
            Some("s1")
        );
        assert!(
            resolve_server_alias(&pool, "my-server", "u2")
                .await
                .unwrap()
                .is_none()
        );
        assert!(!set_default_server(&pool, "u2", "s1").await.unwrap());
        assert!(set_default_server(&pool, "u1", "s1").await.unwrap());
        assert_eq!(
            get_default_server(&pool, "u1").await.unwrap().as_deref(),
            Some("s1")
        );
    }
}
