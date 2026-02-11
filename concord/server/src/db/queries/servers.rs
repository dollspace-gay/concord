use sqlx::SqlitePool;

use crate::db::models::{ServerMemberRow, ServerRow};

/// Create a new server.
pub async fn create_server(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    owner_id: &str,
    icon_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO servers (id, name, owner_id, icon_url) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(name)
        .bind(owner_id)
        .bind(icon_url)
        .execute(pool)
        .await?;

    // Owner is automatically a member with 'owner' role
    sqlx::query("INSERT INTO server_members (server_id, user_id, role) VALUES (?, ?, 'owner')")
        .bind(id)
        .bind(owner_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Get a server by ID.
pub async fn get_server(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Option<ServerRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerRow>("SELECT * FROM servers WHERE id = ?")
        .bind(server_id)
        .fetch_optional(pool)
        .await
}

/// List all servers a user is a member of.
pub async fn list_servers_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<ServerRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerRow>(
        "SELECT s.* FROM servers s \
         JOIN server_members sm ON s.id = sm.server_id \
         WHERE sm.user_id = ? \
         ORDER BY s.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// List all servers (for system admin).
pub async fn list_all_servers(pool: &SqlitePool) -> Result<Vec<ServerRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerRow>("SELECT * FROM servers ORDER BY name")
        .fetch_all(pool)
        .await
}

/// Update a server's name and/or icon.
pub async fn update_server(
    pool: &SqlitePool,
    server_id: &str,
    name: &str,
    icon_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE servers SET name = ?, icon_url = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(name)
    .bind(icon_url)
    .bind(server_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a server and all associated data (cascades).
pub async fn delete_server(pool: &SqlitePool, server_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM servers WHERE id = ?")
        .bind(server_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Add a user to a server.
pub async fn add_server_member(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    role: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO server_members (server_id, user_id, role) VALUES (?, ?, ?)")
        .bind(server_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove a user from a server.
pub async fn remove_server_member(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get a specific server member record.
pub async fn get_server_member(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Option<ServerMemberRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerMemberRow>(
        "SELECT * FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Get all members of a server.
pub async fn get_server_members(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ServerMemberRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerMemberRow>(
        "SELECT * FROM server_members WHERE server_id = ? ORDER BY joined_at",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

/// Update a member's role within a server.
pub async fn update_member_role(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    role: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE server_members SET role = ? WHERE server_id = ? AND user_id = ?")
        .bind(role)
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get the member count for a server.
pub async fn get_member_count(pool: &SqlitePool, server_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM server_members WHERE server_id = ?")
        .bind(server_id)
        .fetch_one(pool)
        .await
}

/// Check if a user is a system admin.
pub async fn is_system_admin(pool: &SqlitePool, user_id: &str) -> Result<bool, sqlx::Error> {
    let val: i32 = sqlx::query_scalar("SELECT is_system_admin FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .unwrap_or(0);
    Ok(val != 0)
}

/// Set or unset system admin flag for a user.
pub async fn set_system_admin(
    pool: &SqlitePool,
    user_id: &str,
    is_admin: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET is_system_admin = ? WHERE id = ?")
        .bind(if is_admin { 1 } else { 0 })
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get a member's server-specific nickname.
pub async fn get_server_nickname(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT nickname FROM server_members WHERE server_id = ? AND user_id = ?")
            .bind(server_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

/// Set a member's server-specific nickname.
pub async fn set_server_nickname(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    nickname: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE server_members SET nickname = ? WHERE server_id = ? AND user_id = ?")
        .bind(nickname)
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::users::{self, CreateOAuthUser};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn create_test_user(pool: &SqlitePool, id: &str, username: &str) {
        users::create_with_oauth(
            pool,
            &CreateOAuthUser {
                user_id: id,
                username,
                email: None,
                avatar_url: None,
                oauth_id: &format!("oauth-{id}"),
                provider: "github",
                provider_id: &format!("gh-{id}"),
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_create_and_get_server() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;

        create_server(&pool, "s1", "Test Server", "u1", None)
            .await
            .unwrap();

        let server = get_server(&pool, "s1").await.unwrap();
        assert!(server.is_some());
        let s = server.unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(s.name, "Test Server");
        assert_eq!(s.owner_id, "u1");
        assert!(s.icon_url.is_none());
    }

    #[tokio::test]
    async fn test_create_server_adds_owner_as_member() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_server(&pool, "s1", "Test Server", "u1", None)
            .await
            .unwrap();

        let member = get_server_member(&pool, "s1", "u1").await.unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().role, "owner");
    }

    #[tokio::test]
    async fn test_list_servers_for_user() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_user(&pool, "u2", "bob").await;

        create_server(&pool, "s1", "Alpha", "u1", None)
            .await
            .unwrap();
        create_server(&pool, "s2", "Beta", "u1", None)
            .await
            .unwrap();
        create_server(&pool, "s3", "Gamma", "u2", None)
            .await
            .unwrap();

        let user1_servers = list_servers_for_user(&pool, "u1").await.unwrap();
        assert_eq!(user1_servers.len(), 2);
        // Ordered by name
        assert_eq!(user1_servers[0].name, "Alpha");
        assert_eq!(user1_servers[1].name, "Beta");

        let user2_servers = list_servers_for_user(&pool, "u2").await.unwrap();
        assert_eq!(user2_servers.len(), 1);
    }

    #[tokio::test]
    async fn test_add_and_remove_member() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_user(&pool, "u2", "bob").await;

        create_server(&pool, "s1", "Test", "u1", None)
            .await
            .unwrap();

        // Add member
        add_server_member(&pool, "s1", "u2", "member")
            .await
            .unwrap();
        let count = get_member_count(&pool, "s1").await.unwrap();
        assert_eq!(count, 2); // owner + new member

        let member = get_server_member(&pool, "s1", "u2").await.unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().role, "member");

        // Remove member
        remove_server_member(&pool, "s1", "u2").await.unwrap();
        let count_after = get_member_count(&pool, "s1").await.unwrap();
        assert_eq!(count_after, 1);
    }

    #[tokio::test]
    async fn test_update_member_role() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_user(&pool, "u2", "bob").await;

        create_server(&pool, "s1", "Test", "u1", None)
            .await
            .unwrap();
        add_server_member(&pool, "s1", "u2", "member")
            .await
            .unwrap();

        update_member_role(&pool, "s1", "u2", "admin")
            .await
            .unwrap();

        let member = get_server_member(&pool, "s1", "u2").await.unwrap().unwrap();
        assert_eq!(member.role, "admin");
    }

    #[tokio::test]
    async fn test_update_server() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_server(&pool, "s1", "Old Name", "u1", None)
            .await
            .unwrap();

        update_server(&pool, "s1", "New Name", Some("https://icon.png"))
            .await
            .unwrap();

        let server = get_server(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(server.name, "New Name");
        assert_eq!(server.icon_url, Some("https://icon.png".to_string()));
    }

    #[tokio::test]
    async fn test_delete_server() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_server(&pool, "s1", "Test", "u1", None)
            .await
            .unwrap();

        delete_server(&pool, "s1").await.unwrap();

        let server = get_server(&pool, "s1").await.unwrap();
        assert!(server.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_server() {
        let pool = setup_db().await;
        let server = get_server(&pool, "nonexistent").await.unwrap();
        assert!(server.is_none());
    }

    #[tokio::test]
    async fn test_list_all_servers() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;

        create_server(&pool, "s1", "Beta", "u1", None)
            .await
            .unwrap();
        create_server(&pool, "s2", "Alpha", "u1", None)
            .await
            .unwrap();

        let all = list_all_servers(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Alpha"); // ordered by name
    }

    #[tokio::test]
    async fn test_system_admin() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;

        assert!(!is_system_admin(&pool, "u1").await.unwrap());

        set_system_admin(&pool, "u1", true).await.unwrap();
        assert!(is_system_admin(&pool, "u1").await.unwrap());

        set_system_admin(&pool, "u1", false).await.unwrap();
        assert!(!is_system_admin(&pool, "u1").await.unwrap());
    }

    #[tokio::test]
    async fn test_add_member_idempotent() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_user(&pool, "u2", "bob").await;
        create_server(&pool, "s1", "Test", "u1", None)
            .await
            .unwrap();

        add_server_member(&pool, "s1", "u2", "member")
            .await
            .unwrap();
        // Adding again should not error (INSERT OR IGNORE)
        add_server_member(&pool, "s1", "u2", "member")
            .await
            .unwrap();

        let count = get_member_count(&pool, "s1").await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_get_server_members() {
        let pool = setup_db().await;
        create_test_user(&pool, "u1", "alice").await;
        create_test_user(&pool, "u2", "bob").await;
        create_test_user(&pool, "u3", "charlie").await;
        create_server(&pool, "s1", "Test", "u1", None)
            .await
            .unwrap();

        add_server_member(&pool, "s1", "u2", "member")
            .await
            .unwrap();
        add_server_member(&pool, "s1", "u3", "member")
            .await
            .unwrap();

        let members = get_server_members(&pool, "s1").await.unwrap();
        assert_eq!(members.len(), 3);
    }
}
