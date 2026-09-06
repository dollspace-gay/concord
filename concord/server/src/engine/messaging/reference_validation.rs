use super::{MentionKind, MessageMention, MessagingError, SqliteConnection};

pub(super) async fn validate_reply(
    connection: &mut SqliteConnection,
    conversation_id: &str,
    reply_to_id: Option<&str>,
) -> Result<(), MessagingError> {
    let Some(reply_to_id) = reply_to_id else {
        return Ok(());
    };
    let valid: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM messages \
         WHERE id=? AND conversation_id=? AND deleted_at IS NULL)",
    )
    .bind(reply_to_id)
    .bind(conversation_id)
    .fetch_one(connection)
    .await?;
    if valid == 0 {
        return Err(MessagingError::Unavailable);
    }
    Ok(())
}

pub(super) async fn validate_attachments(
    connection: &mut SqliteConnection,
    user_id: &str,
    conversation_id: &str,
    attachment_ids: &[String],
) -> Result<(), MessagingError> {
    if attachment_ids.is_empty() {
        return Ok(());
    }
    let mut builder =
        sqlx::QueryBuilder::new("SELECT COUNT(*) FROM attachments WHERE uploader_id=");
    builder.push_bind(user_id);
    builder.push(" AND conversation_id=");
    builder.push_bind(conversation_id);
    builder.push(
        " AND media_state='ready' AND storage_backend='local' \
         AND storage_key IS NOT NULL AND message_id IS NULL AND id IN (",
    );
    let mut separated = builder.separated(",");
    for id in attachment_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let count: i64 = builder.build_query_scalar().fetch_one(connection).await?;
    if count != attachment_ids.len() as i64 {
        return Err(MessagingError::Unavailable);
    }
    Ok(())
}

pub(super) async fn validate_mentions(
    connection: &mut SqliteConnection,
    server_id: &str,
    content: &str,
    mentions: &[MessageMention],
) -> Result<(), MessagingError> {
    let mut previous_end = 0;
    for mention in mentions {
        if mention.start_byte < previous_end
            || mention.end_byte > content.len()
            || !content.is_char_boundary(mention.start_byte)
            || !content.is_char_boundary(mention.end_byte)
            || mention.start_byte == mention.end_byte
        {
            return Err(MessagingError::InvalidInput("invalid mention range".into()));
        }
        previous_end = mention.end_byte;
        let exists = match mention.kind {
            MentionKind::Everyone => mention.target_id.is_none(),
            MentionKind::User => {
                let Some(target) = mention.target_id.as_deref() else {
                    return Err(MessagingError::InvalidInput(
                        "user mention requires a target".into(),
                    ));
                };
                sqlx::query_scalar::<_, i64>(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
                )
                .bind(server_id)
                .bind(target)
                .fetch_one(&mut *connection)
                .await?
                    != 0
            }
            MentionKind::Role => {
                let Some(target) = mention.target_id.as_deref() else {
                    return Err(MessagingError::InvalidInput(
                        "role mention requires a target".into(),
                    ));
                };
                sqlx::query_scalar::<_, i64>(
                    "SELECT EXISTS(SELECT 1 FROM roles WHERE server_id=? AND id=?)",
                )
                .bind(server_id)
                .bind(target)
                .fetch_one(&mut *connection)
                .await?
                    != 0
            }
        };
        if !exists {
            return Err(MessagingError::Unavailable);
        }
    }
    Ok(())
}
