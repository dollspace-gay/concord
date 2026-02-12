use sqlx::SqlitePool;

use crate::db::models::MessageRow;

/// Sanitize a search query for FTS5 MATCH by quoting each term.
/// This prevents FTS5 operator injection (AND, OR, NOT, NEAR, *, etc.).
fn sanitize_fts5_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| {
            // Escape internal double quotes and wrap each term
            let escaped = term.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Full-text search messages within a server, optionally filtered by channel.
pub async fn search_messages(
    pool: &SqlitePool,
    server_id: &str,
    query: &str,
    channel_id: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<MessageRow>, i64), sqlx::Error> {
    // Sanitize the query to prevent FTS5 operator injection
    let safe_query = sanitize_fts5_query(query);

    // Use FTS5 MATCH for full-text search
    let (rows, total) = if let Some(ch_id) = channel_id {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.server_id, m.channel_id, m.sender_id, m.sender_nick, m.content, \
             m.created_at, m.target_user_id, m.edited_at, m.deleted_at, m.reply_to_id \
             FROM messages m \
             JOIN messages_fts f ON m.rowid = f.rowid \
             WHERE f.content MATCH ? AND m.server_id = ? AND m.channel_id = ? AND m.deleted_at IS NULL \
             ORDER BY m.created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(&safe_query)
        .bind(server_id)
        .bind(ch_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messages m \
             JOIN messages_fts f ON m.rowid = f.rowid \
             WHERE f.content MATCH ? AND m.server_id = ? AND m.channel_id = ? AND m.deleted_at IS NULL",
        )
        .bind(&safe_query)
        .bind(server_id)
        .bind(ch_id)
        .fetch_one(pool)
        .await?;

        (rows, total.0)
    } else {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.server_id, m.channel_id, m.sender_id, m.sender_nick, m.content, \
             m.created_at, m.target_user_id, m.edited_at, m.deleted_at, m.reply_to_id \
             FROM messages m \
             JOIN messages_fts f ON m.rowid = f.rowid \
             WHERE f.content MATCH ? AND m.server_id = ? AND m.deleted_at IS NULL \
             ORDER BY m.created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(&safe_query)
        .bind(server_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messages m \
             JOIN messages_fts f ON m.rowid = f.rowid \
             WHERE f.content MATCH ? AND m.server_id = ? AND m.deleted_at IS NULL",
        )
        .bind(&safe_query)
        .bind(server_id)
        .fetch_one(pool)
        .await?;

        (rows, total.0)
    };

    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::db::queries::channels;
    use crate::db::queries::messages::{self, InsertMessageParams};
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
        channels::ensure_channel(pool, "c2", "s1", "#random")
            .await
            .unwrap();
    }

    async fn insert_msg(pool: &SqlitePool, id: &str, channel_id: &str, content: &str) {
        messages::insert_message(
            pool,
            &InsertMessageParams {
                id,
                server_id: "s1",
                channel_id,
                sender_id: "u1",
                sender_nick: "alice",
                content,
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_search_messages_basic() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        insert_msg(&pool, "m1", "c1", "hello world").await;
        insert_msg(&pool, "m2", "c1", "goodbye world").await;
        insert_msg(&pool, "m3", "c1", "something else").await;

        let (results, total) = search_messages(&pool, "s1", "world", None, 50, 0)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_with_channel_filter() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        insert_msg(&pool, "m1", "c1", "hello world").await;
        insert_msg(&pool, "m2", "c2", "hello world too").await;

        let (results, total) = search_messages(&pool, "s1", "hello", Some("c1"), 50, 0)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].channel_id, Some("c1".to_string()));
    }

    #[tokio::test]
    async fn test_search_excludes_deleted() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        insert_msg(&pool, "m1", "c1", "find me").await;
        insert_msg(&pool, "m2", "c1", "find me too").await;

        messages::soft_delete_message(&pool, "m2").await.unwrap();

        let (results, total) = search_messages(&pool, "s1", "find", None, 50, 0)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_with_pagination() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        for i in 0..5 {
            insert_msg(&pool, &format!("m{i}"), "c1", "searchable content").await;
        }

        let (results, total) = search_messages(&pool, "s1", "searchable", None, 2, 0)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(results.len(), 2);

        let (results2, _) = search_messages(&pool, "s1", "searchable", None, 2, 2)
            .await
            .unwrap();
        assert_eq!(results2.len(), 2);
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        insert_msg(&pool, "m1", "c1", "hello world").await;

        let (results, total) = search_messages(&pool, "s1", "nonexistent", None, 50, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(results.is_empty());
    }
}
