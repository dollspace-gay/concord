use sqlx::SqlitePool;

use crate::db::models::{CreateWebhookParams, WebhookEventRow, WebhookRow};

pub async fn create_webhook(
    pool: &SqlitePool,
    p: &CreateWebhookParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO webhooks (id, server_id, channel_id, name, avatar_url, webhook_type, token, url, created_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(p.id)
    .bind(p.server_id)
    .bind(p.channel_id)
    .bind(p.name)
    .bind(p.avatar_url)
    .bind(p.webhook_type)
    .bind(p.token)
    .bind(p.url)
    .bind(p.created_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_webhook(
    pool: &SqlitePool,
    webhook_id: &str,
) -> Result<Option<WebhookRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE id = ?")
        .bind(webhook_id)
        .fetch_optional(pool)
        .await
}

pub async fn get_webhook_by_token(
    pool: &SqlitePool,
    token: &str,
) -> Result<Option<WebhookRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE token = ?")
        .bind(token)
        .fetch_optional(pool)
        .await
}

pub async fn list_webhooks(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<Vec<WebhookRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookRow>(
        "SELECT * FROM webhooks WHERE server_id = ? ORDER BY created_at DESC",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn list_channel_webhooks(
    pool: &SqlitePool,
    channel_id: &str,
) -> Result<Vec<WebhookRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookRow>(
        "SELECT * FROM webhooks WHERE channel_id = ? ORDER BY created_at DESC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
}

pub async fn update_webhook(
    pool: &SqlitePool,
    webhook_id: &str,
    name: &str,
    avatar_url: Option<&str>,
    channel_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE webhooks SET name = ?, avatar_url = ?, channel_id = ? WHERE id = ?")
        .bind(name)
        .bind(avatar_url)
        .bind(channel_id)
        .bind(webhook_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_webhook(pool: &SqlitePool, webhook_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM webhooks WHERE id = ?")
        .bind(webhook_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn add_webhook_event(
    pool: &SqlitePool,
    id: &str,
    webhook_id: &str,
    event_type: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO webhook_events (id, webhook_id, event_type) VALUES (?, ?, ?)",
    )
    .bind(id)
    .bind(webhook_id)
    .bind(event_type)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_webhook_event(
    pool: &SqlitePool,
    webhook_id: &str,
    event_type: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM webhook_events WHERE webhook_id = ? AND event_type = ?")
        .bind(webhook_id)
        .bind(event_type)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_webhook_events(
    pool: &SqlitePool,
    webhook_id: &str,
) -> Result<Vec<WebhookEventRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookEventRow>("SELECT * FROM webhook_events WHERE webhook_id = ?")
        .bind(webhook_id)
        .fetch_all(pool)
        .await
}

pub async fn list_outgoing_webhooks_for_event(
    pool: &SqlitePool,
    server_id: &str,
    event_type: &str,
) -> Result<Vec<WebhookRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookRow>(
        "SELECT w.* FROM webhooks w
         JOIN webhook_events we ON we.webhook_id = w.id
         WHERE w.server_id = ? AND w.webhook_type = 'outgoing' AND we.event_type = ?",
    )
    .bind(server_id)
    .bind(event_type)
    .fetch_all(pool)
    .await
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

    async fn setup_env(pool: &SqlitePool) {
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
        channels::ensure_channel(pool, "c1", "s1", "#general")
            .await
            .unwrap();
    }

    fn wh_params<'a>(id: &'a str, token: &'a str) -> CreateWebhookParams<'a> {
        CreateWebhookParams {
            id,
            server_id: "s1",
            channel_id: "c1",
            name: "Test Webhook",
            avatar_url: None,
            webhook_type: "incoming",
            token,
            url: None,
            created_by: "u1",
        }
    }

    #[tokio::test]
    async fn test_create_and_get_webhook() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();

        let wh = get_webhook(&pool, "w1").await.unwrap();
        assert!(wh.is_some());
        let w = wh.unwrap();
        assert_eq!(w.name, "Test Webhook");
        assert_eq!(w.token, "tok1");
        assert_eq!(w.webhook_type, "incoming");
    }

    #[tokio::test]
    async fn test_get_webhook_by_token() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "secret-tok"))
            .await
            .unwrap();

        let wh = get_webhook_by_token(&pool, "secret-tok").await.unwrap();
        assert!(wh.is_some());
        assert_eq!(wh.unwrap().id, "w1");

        let not_found = get_webhook_by_token(&pool, "wrong-tok").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_list_webhooks_by_server() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();
        create_webhook(&pool, &wh_params("w2", "tok2"))
            .await
            .unwrap();

        let webhooks = list_webhooks(&pool, "s1").await.unwrap();
        assert_eq!(webhooks.len(), 2);
    }

    #[tokio::test]
    async fn test_list_channel_webhooks() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();

        let webhooks = list_channel_webhooks(&pool, "c1").await.unwrap();
        assert_eq!(webhooks.len(), 1);

        let empty = list_channel_webhooks(&pool, "nonexistent").await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_update_webhook() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();

        update_webhook(
            &pool,
            "w1",
            "Updated Name",
            Some("https://avatar.png"),
            "c1",
        )
        .await
        .unwrap();

        let wh = get_webhook(&pool, "w1").await.unwrap().unwrap();
        assert_eq!(wh.name, "Updated Name");
        assert_eq!(wh.avatar_url, Some("https://avatar.png".to_string()));
    }

    #[tokio::test]
    async fn test_delete_webhook() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();

        delete_webhook(&pool, "w1").await.unwrap();

        let wh = get_webhook(&pool, "w1").await.unwrap();
        assert!(wh.is_none());
    }

    #[tokio::test]
    async fn test_webhook_events() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();

        add_webhook_event(&pool, "we1", "w1", "message_create")
            .await
            .unwrap();
        add_webhook_event(&pool, "we2", "w1", "member_join")
            .await
            .unwrap();

        let events = list_webhook_events(&pool, "w1").await.unwrap();
        assert_eq!(events.len(), 2);

        remove_webhook_event(&pool, "w1", "message_create")
            .await
            .unwrap();
        let events = list_webhook_events(&pool, "w1").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "member_join");
    }

    #[tokio::test]
    async fn test_webhook_event_idempotent() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        create_webhook(&pool, &wh_params("w1", "tok1"))
            .await
            .unwrap();

        add_webhook_event(&pool, "we1", "w1", "message_create")
            .await
            .unwrap();
        add_webhook_event(&pool, "we1", "w1", "message_create")
            .await
            .unwrap(); // INSERT OR IGNORE

        let events = list_webhook_events(&pool, "w1").await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_list_outgoing_webhooks_for_event() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        create_webhook(
            &pool,
            &CreateWebhookParams {
                id: "w1",
                server_id: "s1",
                channel_id: "c1",
                name: "Outgoing",
                avatar_url: None,
                webhook_type: "outgoing",
                token: "tok1",
                url: Some("https://example.com/hook"),
                created_by: "u1",
            },
        )
        .await
        .unwrap();
        add_webhook_event(&pool, "we1", "w1", "message_create")
            .await
            .unwrap();

        let hooks = list_outgoing_webhooks_for_event(&pool, "s1", "message_create")
            .await
            .unwrap();
        assert_eq!(hooks.len(), 1);

        let none = list_outgoing_webhooks_for_event(&pool, "s1", "other_event")
            .await
            .unwrap();
        assert!(none.is_empty());
    }
}
