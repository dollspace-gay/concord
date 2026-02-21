use sqlx::SqlitePool;

/// A stored attachment record from the database.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub uploader_id: String,
    pub message_id: Option<String>,
    pub filename: String,
    pub original_filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub created_at: String,
    pub blob_cid: Option<String>,
    pub blob_url: Option<String>,
}

/// Insert a new attachment record.
pub async fn insert_attachment(
    pool: &SqlitePool,
    id: &str,
    uploader_id: &str,
    filename: &str,
    original_filename: &str,
    content_type: &str,
    file_size: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO attachments (id, uploader_id, filename, original_filename, content_type, file_size) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(uploader_id)
    .bind(filename)
    .bind(original_filename)
    .bind(content_type)
    .bind(file_size)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a single attachment by ID.
pub async fn get_attachment(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, uploader_id, message_id, filename, original_filename, content_type, file_size, created_at, blob_cid, blob_url \
         FROM attachments WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Get attachments by a list of IDs.
pub async fn get_attachments_by_ids(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT id, uploader_id, message_id, filename, original_filename, content_type, file_size, created_at, blob_cid, blob_url \
         FROM attachments WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, AttachmentRow>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await
}

/// Get all attachments linked to a specific message.
pub async fn get_attachments_for_message(
    pool: &SqlitePool,
    message_id: &str,
) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, uploader_id, message_id, filename, original_filename, content_type, file_size, created_at, blob_cid, blob_url \
         FROM attachments WHERE message_id = ? ORDER BY created_at",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await
}

/// Get all attachments for a batch of message IDs.
pub async fn get_attachments_for_messages(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<&str> = message_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT id, uploader_id, message_id, filename, original_filename, content_type, file_size, created_at, blob_cid, blob_url \
         FROM attachments WHERE message_id IN ({}) ORDER BY created_at",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, AttachmentRow>(&sql);
    for id in message_ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await
}

/// Parameters for inserting an attachment with a PDS blob reference.
pub struct InsertBlobAttachmentParams<'a> {
    pub id: &'a str,
    pub uploader_id: &'a str,
    pub filename: &'a str,
    pub original_filename: &'a str,
    pub content_type: &'a str,
    pub file_size: i64,
    pub blob_cid: &'a str,
    pub blob_url: &'a str,
}

/// Insert a new attachment record with PDS blob reference.
pub async fn insert_attachment_with_blob(
    pool: &SqlitePool,
    params: &InsertBlobAttachmentParams<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO attachments (id, uploader_id, filename, original_filename, content_type, file_size, blob_cid, blob_url) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(params.id)
    .bind(params.uploader_id)
    .bind(params.filename)
    .bind(params.original_filename)
    .bind(params.content_type)
    .bind(params.file_size)
    .bind(params.blob_cid)
    .bind(params.blob_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Link attachments to a message (set message_id on matching attachment rows).
pub async fn link_attachments_to_message(
    pool: &SqlitePool,
    message_id: &str,
    attachment_ids: &[String],
    uploader_id: &str,
) -> Result<(), sqlx::Error> {
    for att_id in attachment_ids {
        sqlx::query("UPDATE attachments SET message_id = ? WHERE id = ? AND uploader_id = ?")
            .bind(message_id)
            .bind(att_id)
            .bind(uploader_id)
            .execute(pool)
            .await?;
    }
    Ok(())
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
    }

    #[tokio::test]
    async fn test_insert_and_get_attachment() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        insert_attachment(
            &pool,
            "a1",
            "u1",
            "abc123.png",
            "photo.png",
            "image/png",
            12345,
        )
        .await
        .unwrap();

        let att = get_attachment(&pool, "a1").await.unwrap();
        assert!(att.is_some());
        let a = att.unwrap();
        assert_eq!(a.filename, "abc123.png");
        assert_eq!(a.original_filename, "photo.png");
        assert_eq!(a.content_type, "image/png");
        assert_eq!(a.file_size, 12345);
        assert!(a.message_id.is_none());
        assert!(a.blob_cid.is_none());
    }

    #[tokio::test]
    async fn test_link_attachments_to_message() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        insert_attachment(
            &pool,
            "a1",
            "u1",
            "file1.png",
            "file1.png",
            "image/png",
            100,
        )
        .await
        .unwrap();
        insert_attachment(
            &pool,
            "a2",
            "u1",
            "file2.pdf",
            "file2.pdf",
            "application/pdf",
            200,
        )
        .await
        .unwrap();

        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "See attachments",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        link_attachments_to_message(&pool, "m1", &["a1".to_string(), "a2".to_string()], "u1")
            .await
            .unwrap();

        let att = get_attachment(&pool, "a1").await.unwrap().unwrap();
        assert_eq!(att.message_id, Some("m1".to_string()));
    }

    #[tokio::test]
    async fn test_get_attachments_for_message() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        insert_attachment(&pool, "a1", "u1", "f1.png", "f1.png", "image/png", 100)
            .await
            .unwrap();
        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Test",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
        link_attachments_to_message(&pool, "m1", &["a1".to_string()], "u1")
            .await
            .unwrap();

        let atts = get_attachments_for_message(&pool, "m1").await.unwrap();
        assert_eq!(atts.len(), 1);
    }

    #[tokio::test]
    async fn test_get_attachments_by_ids() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        insert_attachment(&pool, "a1", "u1", "f1.png", "f1.png", "image/png", 100)
            .await
            .unwrap();
        insert_attachment(&pool, "a2", "u1", "f2.png", "f2.png", "image/png", 200)
            .await
            .unwrap();

        let atts = get_attachments_by_ids(&pool, &["a1".to_string(), "a2".to_string()])
            .await
            .unwrap();
        assert_eq!(atts.len(), 2);
    }

    #[tokio::test]
    async fn test_get_attachments_by_ids_empty() {
        let pool = setup_db().await;
        let atts = get_attachments_by_ids(&pool, &[]).await.unwrap();
        assert!(atts.is_empty());
    }

    #[tokio::test]
    async fn test_get_attachments_for_messages() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        insert_attachment(&pool, "a1", "u1", "f1.png", "f1.png", "image/png", 100)
            .await
            .unwrap();
        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Test",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();
        link_attachments_to_message(&pool, "m1", &["a1".to_string()], "u1")
            .await
            .unwrap();

        let atts = get_attachments_for_messages(&pool, &["m1".to_string()])
            .await
            .unwrap();
        assert_eq!(atts.len(), 1);

        let empty = get_attachments_for_messages(&pool, &[]).await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_insert_attachment_with_blob() {
        let pool = setup_db().await;
        setup_env(&pool).await;

        insert_attachment_with_blob(
            &pool,
            &InsertBlobAttachmentParams {
                id: "a1",
                uploader_id: "u1",
                filename: "abc.png",
                original_filename: "photo.png",
                content_type: "image/png",
                file_size: 5000,
                blob_cid: "bafyreig...",
                blob_url: "https://pds.example/blob/...",
            },
        )
        .await
        .unwrap();

        let att = get_attachment(&pool, "a1").await.unwrap().unwrap();
        assert_eq!(att.blob_cid, Some("bafyreig...".to_string()));
        assert_eq!(
            att.blob_url,
            Some("https://pds.example/blob/...".to_string())
        );
    }

    #[tokio::test]
    async fn test_link_wrong_uploader() {
        let pool = setup_db().await;
        setup_env(&pool).await;
        insert_attachment(&pool, "a1", "u1", "f.png", "f.png", "image/png", 100)
            .await
            .unwrap();
        messages::insert_message(
            &pool,
            &InsertMessageParams {
                id: "m1",
                server_id: "s1",
                channel_id: "c1",
                sender_id: "u1",
                sender_nick: "alice",
                content: "Test",
                reply_to_id: None,
            },
        )
        .await
        .unwrap();

        // Try linking with wrong uploader -- should not update
        link_attachments_to_message(&pool, "m1", &["a1".to_string()], "wrong-user")
            .await
            .unwrap();

        let att = get_attachment(&pool, "a1").await.unwrap().unwrap();
        assert!(att.message_id.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_attachment() {
        let pool = setup_db().await;
        let att = get_attachment(&pool, "nosuch").await.unwrap();
        assert!(att.is_none());
    }
}
