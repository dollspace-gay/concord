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
    sqlx::query(
        "INSERT INTO servers (id, name, owner_id, icon_url) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(owner_id)
    .bind(icon_url)
    .execute(pool)
    .await?;

    // Owner is automatically a member with 'owner' role
    sqlx::query(
        "INSERT INTO server_members (server_id, user_id, role) VALUES (?, ?, 'owner')",
    )
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
    sqlx::query(
        "INSERT OR IGNORE INTO server_members (server_id, user_id, role) VALUES (?, ?, ?)",
    )
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
pub async fn get_member_count(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<i64, sqlx::Error> {
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
