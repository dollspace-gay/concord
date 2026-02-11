use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EmbedRow {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub site_name: Option<String>,
}

pub async fn get_cached_embed(
    pool: &SqlitePool,
    url: &str,
) -> Result<Option<EmbedRow>, sqlx::Error> {
    let row = sqlx::query_as::<_, EmbedRow>(
        "SELECT url, title, description, image_url, site_name \
         FROM embed_cache \
         WHERE url = ? AND datetime(fetched_at) > datetime('now', '-24 hours')",
    )
    .bind(url)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn upsert_embed(
    pool: &SqlitePool,
    url: &str,
    title: Option<&str>,
    description: Option<&str>,
    image_url: Option<&str>,
    site_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO embed_cache (url, title, description, image_url, site_name, fetched_at) \
         VALUES (?, ?, ?, ?, ?, datetime('now')) \
         ON CONFLICT(url) DO UPDATE SET \
           title = excluded.title, \
           description = excluded.description, \
           image_url = excluded.image_url, \
           site_name = excluded.site_name, \
           fetched_at = excluded.fetched_at",
    )
    .bind(url)
    .bind(title)
    .bind(description)
    .bind(image_url)
    .bind(site_name)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};

    async fn setup_db() -> SqlitePool {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_upsert_and_get_cached_embed() {
        let pool = setup_db().await;

        upsert_embed(
            &pool,
            "https://example.com",
            Some("Example"),
            Some("An example site"),
            Some("https://example.com/img.png"),
            Some("Example.com"),
        )
        .await
        .unwrap();

        let embed = get_cached_embed(&pool, "https://example.com")
            .await
            .unwrap();
        assert!(embed.is_some());
        let e = embed.unwrap();
        assert_eq!(e.url, "https://example.com");
        assert_eq!(e.title, Some("Example".to_string()));
        assert_eq!(e.description, Some("An example site".to_string()));
        assert_eq!(e.site_name, Some("Example.com".to_string()));
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let pool = setup_db().await;

        upsert_embed(
            &pool,
            "https://example.com",
            Some("Old Title"),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        upsert_embed(
            &pool,
            "https://example.com",
            Some("New Title"),
            Some("New desc"),
            None,
            None,
        )
        .await
        .unwrap();

        let embed = get_cached_embed(&pool, "https://example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(embed.title, Some("New Title".to_string()));
        assert_eq!(embed.description, Some("New desc".to_string()));
    }

    #[tokio::test]
    async fn test_get_nonexistent_embed() {
        let pool = setup_db().await;
        let embed = get_cached_embed(&pool, "https://nosuch.example")
            .await
            .unwrap();
        assert!(embed.is_none());
    }

    #[tokio::test]
    async fn test_embed_with_null_fields() {
        let pool = setup_db().await;

        upsert_embed(&pool, "https://minimal.com", None, None, None, None)
            .await
            .unwrap();

        let embed = get_cached_embed(&pool, "https://minimal.com")
            .await
            .unwrap()
            .unwrap();
        assert!(embed.title.is_none());
        assert!(embed.description.is_none());
        assert!(embed.image_url.is_none());
        assert!(embed.site_name.is_none());
    }
}
