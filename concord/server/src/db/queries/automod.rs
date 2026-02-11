use sqlx::SqlitePool;

use crate::db::models::{AutomodRuleRow, CreateAutomodRuleParams};

pub async fn create_rule(
    pool: &SqlitePool,
    params: &CreateAutomodRuleParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO automod_rules (id, server_id, name, rule_type, config, action_type, timeout_duration_seconds) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(params.id)
    .bind(params.server_id)
    .bind(params.name)
    .bind(params.rule_type)
    .bind(params.config)
    .bind(params.action_type)
    .bind(params.timeout_duration_seconds)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_rule(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    enabled: bool,
    config: &str,
    action_type: &str,
    timeout_duration_seconds: Option<i32>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE automod_rules SET name = ?, enabled = ?, config = ?, action_type = ?, timeout_duration_seconds = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(name)
    .bind(enabled as i32)
    .bind(config)
    .bind(action_type)
    .bind(timeout_duration_seconds)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_rule(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM automod_rules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_rules(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<AutomodRuleRow>, sqlx::Error> {
    sqlx::query_as::<_, AutomodRuleRow>(
        "SELECT * FROM automod_rules WHERE server_id = ? ORDER BY created_at",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn get_enabled_rules(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<AutomodRuleRow>, sqlx::Error> {
    sqlx::query_as::<_, AutomodRuleRow>(
        "SELECT * FROM automod_rules WHERE server_id = ? AND enabled = 1 ORDER BY created_at",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
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
    async fn test_create_and_list_rules() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am1",
                server_id: "s1",
                name: "No Spam",
                rule_type: "keyword",
                config: "{\"words\":[\"spam\"]}",
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();

        let rules = list_rules(&pool, "s1").await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "No Spam");
        assert_eq!(rules[0].enabled, 1); // default enabled
    }

    #[tokio::test]
    async fn test_update_rule() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am1",
                server_id: "s1",
                name: "Old Name",
                rule_type: "keyword",
                config: "{}",
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();

        let updated = update_rule(
            &pool,
            "am1",
            "New Name",
            false,
            "{\"new\":true}",
            "timeout",
            Some(300),
        )
        .await
        .unwrap();
        assert!(updated);

        let rules = list_rules(&pool, "s1").await.unwrap();
        assert_eq!(rules[0].name, "New Name");
        assert_eq!(rules[0].enabled, 0);
        assert_eq!(rules[0].action_type, "timeout");
        assert_eq!(rules[0].timeout_duration_seconds, Some(300));
    }

    #[tokio::test]
    async fn test_delete_rule() {
        let pool = setup_db().await;
        setup_server(&pool).await;
        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am1",
                server_id: "s1",
                name: "Rule",
                rule_type: "keyword",
                config: "{}",
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();

        let deleted = delete_rule(&pool, "am1").await.unwrap();
        assert!(deleted);

        let rules = list_rules(&pool, "s1").await.unwrap();
        assert!(rules.is_empty());

        // Delete nonexistent
        let deleted_again = delete_rule(&pool, "am1").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_get_enabled_rules() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am1",
                server_id: "s1",
                name: "Enabled",
                rule_type: "keyword",
                config: "{}",
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();
        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am2",
                server_id: "s1",
                name: "To Disable",
                rule_type: "mention_spam",
                config: "{}",
                action_type: "flag",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();

        // Disable the second rule
        update_rule(&pool, "am2", "Disabled", false, "{}", "flag", None)
            .await
            .unwrap();

        let enabled = get_enabled_rules(&pool, "s1").await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "Enabled");
    }

    #[tokio::test]
    async fn test_rule_types() {
        let pool = setup_db().await;
        setup_server(&pool).await;

        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am1",
                server_id: "s1",
                name: "Keyword Filter",
                rule_type: "keyword",
                config: "{\"words\":[\"bad\"]}",
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();
        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am2",
                server_id: "s1",
                name: "Mention Spam",
                rule_type: "mention_spam",
                config: "{\"max_mentions\":5}",
                action_type: "timeout",
                timeout_duration_seconds: Some(60),
            },
        )
        .await
        .unwrap();
        create_rule(
            &pool,
            &CreateAutomodRuleParams {
                id: "am3",
                server_id: "s1",
                name: "Link Filter",
                rule_type: "link_filter",
                config: "{\"block_all\":true}",
                action_type: "delete",
                timeout_duration_seconds: None,
            },
        )
        .await
        .unwrap();

        let rules = list_rules(&pool, "s1").await.unwrap();
        assert_eq!(rules.len(), 3);
    }
}
