use sqlx::SqlitePool;

use crate::db::models::{RoleRow, UserRoleRow};

/// Parameters for creating a new role (avoids too-many-arguments warning).
pub struct CreateRoleParams<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub color: Option<&'a str>,
    pub icon_url: Option<&'a str>,
    pub position: i32,
    pub permissions: i64,
    pub is_default: bool,
}

/// Create a new role in a server.
pub async fn create_role(
    pool: &SqlitePool,
    params: &CreateRoleParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO roles (id, server_id, name, color, icon_url, position, permissions, is_default) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(params.id)
    .bind(params.server_id)
    .bind(params.name)
    .bind(params.color)
    .bind(params.icon_url)
    .bind(params.position)
    .bind(params.permissions)
    .bind(params.is_default as i32)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a role by ID.
pub async fn get_role(pool: &SqlitePool, role_id: &str) -> Result<Option<RoleRow>, sqlx::Error> {
    sqlx::query_as::<_, RoleRow>("SELECT * FROM roles WHERE id = ?")
        .bind(role_id)
        .fetch_optional(pool)
        .await
}

/// List all roles in a server, ordered by position descending.
pub async fn list_roles(pool: &SqlitePool, server_id: &str) -> Result<Vec<RoleRow>, sqlx::Error> {
    sqlx::query_as::<_, RoleRow>("SELECT * FROM roles WHERE server_id = ? ORDER BY position DESC")
        .bind(server_id)
        .fetch_all(pool)
        .await
}

/// Get the default (@everyone) role for a server.
pub async fn get_default_role(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Option<RoleRow>, sqlx::Error> {
    sqlx::query_as::<_, RoleRow>("SELECT * FROM roles WHERE server_id = ? AND is_default = 1")
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map(|mut rows| rows.pop())
}

/// Update a role's properties.
pub async fn update_role(
    pool: &SqlitePool,
    role_id: &str,
    name: &str,
    color: Option<&str>,
    icon_url: Option<&str>,
    permissions: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE roles SET name = ?, color = ?, icon_url = ?, permissions = ? WHERE id = ?")
        .bind(name)
        .bind(color)
        .bind(icon_url)
        .bind(permissions)
        .bind(role_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update a role's position.
pub async fn update_role_position(
    pool: &SqlitePool,
    role_id: &str,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE roles SET position = ? WHERE id = ?")
        .bind(position)
        .bind(role_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a role by ID.
pub async fn delete_role(pool: &SqlitePool, role_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM roles WHERE id = ?")
        .bind(role_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Assign a role to a user in a server.
pub async fn assign_role(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    role_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO user_roles (server_id, user_id, role_id) VALUES (?, ?, ?)")
        .bind(server_id)
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove a role from a user in a server.
pub async fn remove_role(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
    role_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM user_roles WHERE server_id = ? AND user_id = ? AND role_id = ?")
        .bind(server_id)
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get all roles assigned to a user in a server.
pub async fn get_user_roles(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<Vec<RoleRow>, sqlx::Error> {
    sqlx::query_as::<_, RoleRow>(
        "SELECT r.* FROM roles r \
         JOIN user_roles ur ON r.id = ur.role_id \
         WHERE ur.server_id = ? AND ur.user_id = ? \
         ORDER BY r.position DESC",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Get all user-role assignments for a server (for bulk loading).
pub async fn get_all_user_roles(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<UserRoleRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRoleRow>("SELECT * FROM user_roles WHERE server_id = ?")
        .bind(server_id)
        .fetch_all(pool)
        .await
}

/// Check if a server has any roles defined.
pub async fn server_has_roles(pool: &SqlitePool, server_id: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE server_id = ?")
        .bind(server_id)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
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

    async fn setup_user_and_server(pool: &SqlitePool) {
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

    fn default_params<'a>(id: &'a str, name: &'a str) -> CreateRoleParams<'a> {
        CreateRoleParams {
            id,
            server_id: "s1",
            name,
            color: None,
            icon_url: None,
            position: 0,
            permissions: 0,
            is_default: false,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_role() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;

        create_role(&pool, &default_params("r1", "Moderator"))
            .await
            .unwrap();

        let role = get_role(&pool, "r1").await.unwrap();
        assert!(role.is_some());
        let r = role.unwrap();
        assert_eq!(r.id, "r1");
        assert_eq!(r.name, "Moderator");
        assert_eq!(r.server_id, "s1");
        assert_eq!(r.is_default, 0);
    }

    #[tokio::test]
    async fn test_list_roles_ordered_by_position() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;

        create_role(
            &pool,
            &CreateRoleParams {
                id: "r1",
                server_id: "s1",
                name: "Low",
                color: None,
                icon_url: None,
                position: 1,
                permissions: 0,
                is_default: false,
            },
        )
        .await
        .unwrap();
        create_role(
            &pool,
            &CreateRoleParams {
                id: "r2",
                server_id: "s1",
                name: "High",
                color: None,
                icon_url: None,
                position: 10,
                permissions: 0,
                is_default: false,
            },
        )
        .await
        .unwrap();

        let roles = list_roles(&pool, "s1").await.unwrap();
        assert_eq!(roles.len(), 2);
        // Ordered by position DESC
        assert_eq!(roles[0].name, "High");
        assert_eq!(roles[1].name, "Low");
    }

    #[tokio::test]
    async fn test_default_role() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;

        create_role(
            &pool,
            &CreateRoleParams {
                id: "r_default",
                server_id: "s1",
                name: "@everyone",
                color: None,
                icon_url: None,
                position: 0,
                permissions: 0x1,
                is_default: true,
            },
        )
        .await
        .unwrap();

        let def = get_default_role(&pool, "s1").await.unwrap();
        assert!(def.is_some());
        assert_eq!(def.unwrap().name, "@everyone");
    }

    #[tokio::test]
    async fn test_update_role() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;
        create_role(&pool, &default_params("r1", "Old"))
            .await
            .unwrap();

        update_role(&pool, "r1", "New", Some("#ff0000"), None, 0xFF)
            .await
            .unwrap();

        let r = get_role(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(r.name, "New");
        assert_eq!(r.color, Some("#ff0000".to_string()));
        assert_eq!(r.permissions, 0xFF);
    }

    #[tokio::test]
    async fn test_update_role_position() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;
        create_role(&pool, &default_params("r1", "Role1"))
            .await
            .unwrap();

        update_role_position(&pool, "r1", 42).await.unwrap();

        let r = get_role(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(r.position, 42);
    }

    #[tokio::test]
    async fn test_delete_role() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;
        create_role(&pool, &default_params("r1", "ToDelete"))
            .await
            .unwrap();

        delete_role(&pool, "r1").await.unwrap();

        let r = get_role(&pool, "r1").await.unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn test_assign_and_remove_role() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;
        create_role(&pool, &default_params("r1", "Mod"))
            .await
            .unwrap();

        assign_role(&pool, "s1", "u1", "r1").await.unwrap();

        let user_roles = get_user_roles(&pool, "s1", "u1").await.unwrap();
        assert_eq!(user_roles.len(), 1);
        assert_eq!(user_roles[0].name, "Mod");

        remove_role(&pool, "s1", "u1", "r1").await.unwrap();

        let user_roles = get_user_roles(&pool, "s1", "u1").await.unwrap();
        assert!(user_roles.is_empty());
    }

    #[tokio::test]
    async fn test_assign_role_idempotent() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;
        create_role(&pool, &default_params("r1", "Mod"))
            .await
            .unwrap();

        assign_role(&pool, "s1", "u1", "r1").await.unwrap();
        assign_role(&pool, "s1", "u1", "r1").await.unwrap(); // Should not error

        let user_roles = get_user_roles(&pool, "s1", "u1").await.unwrap();
        assert_eq!(user_roles.len(), 1);
    }

    #[tokio::test]
    async fn test_get_all_user_roles() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;
        // Create a second user
        users::create_with_oauth(
            &pool,
            &CreateOAuthUser {
                user_id: "u2",
                username: "bob",
                email: None,
                avatar_url: None,
                oauth_id: "oauth-u2",
                provider: "github",
                provider_id: "gh-u2",
            },
        )
        .await
        .unwrap();
        servers::add_server_member(&pool, "s1", "u2", "member")
            .await
            .unwrap();

        create_role(&pool, &default_params("r1", "Mod"))
            .await
            .unwrap();
        assign_role(&pool, "s1", "u1", "r1").await.unwrap();
        assign_role(&pool, "s1", "u2", "r1").await.unwrap();

        let all = get_all_user_roles(&pool, "s1").await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_server_has_roles() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;

        assert!(!server_has_roles(&pool, "s1").await.unwrap());

        create_role(&pool, &default_params("r1", "Mod"))
            .await
            .unwrap();

        assert!(server_has_roles(&pool, "s1").await.unwrap());
    }

    #[tokio::test]
    async fn test_role_with_color_and_icon() {
        let pool = setup_db().await;
        setup_user_and_server(&pool).await;

        create_role(
            &pool,
            &CreateRoleParams {
                id: "r1",
                server_id: "s1",
                name: "VIP",
                color: Some("#gold"),
                icon_url: Some("https://icon.png"),
                position: 5,
                permissions: 0xABC,
                is_default: false,
            },
        )
        .await
        .unwrap();

        let r = get_role(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(r.color, Some("#gold".to_string()));
        assert_eq!(r.icon_url, Some("https://icon.png".to_string()));
        assert_eq!(r.permissions, 0xABC);
    }
}
