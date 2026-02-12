use sqlx::SqlitePool;

use crate::db::models::{ChannelFollowRow, ServerRow, ServerTemplateRow};

/// List discoverable servers with optional category filter and pagination.
pub async fn list_discoverable_servers(
    pool: &SqlitePool,
    category: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ServerRow>, sqlx::Error> {
    if let Some(cat) = category {
        sqlx::query_as::<_, ServerRow>(
            "SELECT * FROM servers WHERE is_discoverable = 1 AND category = ? ORDER BY name LIMIT ? OFFSET ?",
        )
        .bind(cat)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ServerRow>(
            "SELECT * FROM servers WHERE is_discoverable = 1 ORDER BY name LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }
}

/// Update server community settings.
pub async fn update_server_community(
    pool: &SqlitePool,
    server_id: &str,
    description: Option<&str>,
    is_discoverable: bool,
    welcome_message: Option<&str>,
    rules_text: Option<&str>,
    category: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE servers SET description = ?, is_discoverable = ?, welcome_message = ?, \
         rules_text = ?, category = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(description)
    .bind(if is_discoverable { 1 } else { 0 })
    .bind(welcome_message)
    .bind(rules_text)
    .bind(category)
    .bind(server_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Accept server rules for a member.
pub async fn accept_rules(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE server_members SET rules_accepted = 1 WHERE server_id = ? AND user_id = ?")
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Check if a member has accepted rules.
pub async fn has_accepted_rules(
    pool: &SqlitePool,
    server_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let val: i32 = sqlx::query_scalar(
        "SELECT rules_accepted FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0);
    Ok(val != 0)
}

/// Set channel as announcement channel.
pub async fn set_announcement_channel(
    pool: &SqlitePool,
    channel_id: &str,
    is_announcement: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET is_announcement = ? WHERE id = ?")
        .bind(if is_announcement { 1 } else { 0 })
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Create a channel follow (cross-posting).
pub async fn create_channel_follow(
    pool: &SqlitePool,
    id: &str,
    source_channel_id: &str,
    target_channel_id: &str,
    created_by: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO channel_follows (id, source_channel_id, target_channel_id, created_by) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(source_channel_id)
    .bind(target_channel_id)
    .bind(created_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// List followers of an announcement channel.
pub async fn list_channel_follows(
    pool: &SqlitePool,
    source_channel_id: &str,
) -> Result<Vec<ChannelFollowRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelFollowRow>(
        "SELECT * FROM channel_follows WHERE source_channel_id = ?",
    )
    .bind(source_channel_id)
    .fetch_all(pool)
    .await
}

/// Delete a channel follow.
pub async fn delete_channel_follow(pool: &SqlitePool, follow_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM channel_follows WHERE id = ?")
        .bind(follow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Create a server template.
pub async fn create_template(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    description: Option<&str>,
    server_id: &str,
    created_by: &str,
    config: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO server_templates (id, name, description, server_id, created_by, config) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(server_id)
    .bind(created_by)
    .bind(config)
    .execute(pool)
    .await?;
    Ok(())
}

/// List templates for a server.
pub async fn list_templates(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<ServerTemplateRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerTemplateRow>(
        "SELECT * FROM server_templates WHERE server_id = ? ORDER BY created_at DESC",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

/// Get a specific template.
pub async fn get_template(
    pool: &SqlitePool,
    template_id: &str,
) -> Result<Option<ServerTemplateRow>, sqlx::Error> {
    sqlx::query_as::<_, ServerTemplateRow>("SELECT * FROM server_templates WHERE id = ?")
        .bind(template_id)
        .fetch_optional(pool)
        .await
}

/// Delete a template.
pub async fn delete_template(pool: &SqlitePool, template_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM server_templates WHERE id = ?")
        .bind(template_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Increment template use count.
pub async fn increment_template_use(
    pool: &SqlitePool,
    template_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE server_templates SET use_count = use_count + 1 WHERE id = ?")
        .bind(template_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
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
    async fn test_update_and_list_discoverable() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        // Initially not discoverable
        let discoverable = list_discoverable_servers(&pool, None, 100, 0)
            .await
            .unwrap();
        assert!(discoverable.is_empty());

        // Make server discoverable
        update_server_community(
            &pool,
            "s1",
            Some("A great server"),
            true,
            None,
            None,
            Some("gaming"),
        )
        .await
        .unwrap();

        let discoverable = list_discoverable_servers(&pool, None, 100, 0)
            .await
            .unwrap();
        assert_eq!(discoverable.len(), 1);
        assert_eq!(
            discoverable[0].description,
            Some("A great server".to_string())
        );
    }

    #[tokio::test]
    async fn test_discoverable_with_category_filter() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        // Create second server
        servers::create_server(&pool, "s2", "Server 2", "u1", None)
            .await
            .unwrap();

        update_server_community(&pool, "s1", None, true, None, None, Some("gaming"))
            .await
            .unwrap();
        update_server_community(&pool, "s2", None, true, None, None, Some("music"))
            .await
            .unwrap();

        let gaming = list_discoverable_servers(&pool, Some("gaming"), 100, 0)
            .await
            .unwrap();
        assert_eq!(gaming.len(), 1);
        assert_eq!(gaming[0].id, "s1");

        let music = list_discoverable_servers(&pool, Some("music"), 100, 0)
            .await
            .unwrap();
        assert_eq!(music.len(), 1);
        assert_eq!(music[0].id, "s2");
    }

    #[tokio::test]
    async fn test_accept_rules() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        update_server_community(&pool, "s1", None, false, None, Some("Be nice"), None)
            .await
            .unwrap();

        assert!(!has_accepted_rules(&pool, "s1", "u1").await.unwrap());

        accept_rules(&pool, "s1", "u1").await.unwrap();

        assert!(has_accepted_rules(&pool, "s1", "u1").await.unwrap());
    }

    #[tokio::test]
    async fn test_announcement_channel_and_follows() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        channels::ensure_channel(&pool, "c1", "s1", "#announcements")
            .await
            .unwrap();
        channels::ensure_channel(&pool, "c2", "s1", "#mirror")
            .await
            .unwrap();

        set_announcement_channel(&pool, "c1", true).await.unwrap();

        let chan = channels::get_channel(&pool, "c1").await.unwrap().unwrap();
        assert_eq!(chan.is_announcement, 1);

        // Create a follow
        create_channel_follow(&pool, "cf1", "c1", "c2", "u1")
            .await
            .unwrap();

        let follows = list_channel_follows(&pool, "c1").await.unwrap();
        assert_eq!(follows.len(), 1);
        assert_eq!(follows[0].target_channel_id, "c2");

        // Delete follow
        delete_channel_follow(&pool, "cf1").await.unwrap();
        let follows = list_channel_follows(&pool, "c1").await.unwrap();
        assert!(follows.is_empty());
    }

    #[tokio::test]
    async fn test_template_crud() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_template(
            &pool,
            "tmpl1",
            "Basic Server",
            Some("A starter template"),
            "s1",
            "u1",
            "{\"channels\":[]}",
        )
        .await
        .unwrap();

        let tmpl = get_template(&pool, "tmpl1").await.unwrap();
        assert!(tmpl.is_some());
        let t = tmpl.unwrap();
        assert_eq!(t.name, "Basic Server");
        assert_eq!(t.use_count, 0);

        // Increment use count
        increment_template_use(&pool, "tmpl1").await.unwrap();
        increment_template_use(&pool, "tmpl1").await.unwrap();

        let t = get_template(&pool, "tmpl1").await.unwrap().unwrap();
        assert_eq!(t.use_count, 2);
    }

    #[tokio::test]
    async fn test_list_templates() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_template(&pool, "tmpl1", "Template 1", None, "s1", "u1", "{}")
            .await
            .unwrap();
        create_template(&pool, "tmpl2", "Template 2", None, "s1", "u1", "{}")
            .await
            .unwrap();

        let templates = list_templates(&pool, "s1").await.unwrap();
        assert_eq!(templates.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_template() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_template(&pool, "tmpl1", "Template", None, "s1", "u1", "{}")
            .await
            .unwrap();

        delete_template(&pool, "tmpl1").await.unwrap();

        let tmpl = get_template(&pool, "tmpl1").await.unwrap();
        assert!(tmpl.is_none());
    }

    #[tokio::test]
    async fn test_community_settings_full() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        update_server_community(
            &pool,
            "s1",
            Some("Best community"),
            true,
            Some("Welcome to our server!"),
            Some("1. Be respectful\n2. No spam"),
            Some("community"),
        )
        .await
        .unwrap();

        let server = servers::get_server(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(server.description, Some("Best community".to_string()));
        assert_eq!(server.is_discoverable, 1);
        assert_eq!(
            server.welcome_message,
            Some("Welcome to our server!".to_string())
        );
        assert_eq!(
            server.rules_text,
            Some("1. Be respectful\n2. No spam".to_string())
        );
        assert_eq!(server.category, Some("community".to_string()));
    }
}
