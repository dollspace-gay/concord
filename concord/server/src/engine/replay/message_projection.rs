use super::{
    ConversationId, DurableAttachmentProjection, DurableMessageProjection, DurableReplyProjection,
    ReplayError, Row, SqliteConnection,
};

pub(super) async fn load_message_projection(
    connection: &mut SqliteConnection,
    message_id: &str,
) -> Result<Option<DurableMessageProjection>, ReplayError> {
    let row = sqlx::query(
        "SELECT id,conversation_id,conversation_sequence,entity_version,sender_id,sender_nick, \
                content,content_format,created_at,edited_at,deleted_at,reply_to_id, \
                rich_embeds_json,components_json \
         FROM messages WHERE id=? AND conversation_id IS NOT NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let deleted = row.get::<Option<String>, _>(10).is_some();
    let candidate_reply_to_id: Option<String> = (!deleted).then(|| row.get(11)).flatten();
    let reply_to = if let Some(reply_id) = candidate_reply_to_id.as_deref() {
        sqlx::query(
            "SELECT id,sender_id,sender_nick,content,deleted_at FROM messages \
             WHERE id=? AND conversation_id=?",
        )
        .bind(reply_id)
        .bind(row.get::<&str, _>(1))
        .fetch_optional(&mut *connection)
        .await?
        .map(|reply| {
            let reply_deleted = reply.get::<Option<String>, _>(4).is_some();
            DurableReplyProjection {
                message_id: reply.get(0),
                sender_id: reply.get(1),
                sender_nick: reply.get(2),
                content: (!reply_deleted).then(|| reply.get(3)),
                deleted: reply_deleted,
            }
        })
    } else {
        None
    };
    // Historical rows may predate the same-conversation write invariant. Do not
    // expose either their target ID or content through an authorized projection.
    let reply_to_id = reply_to
        .as_ref()
        .map(|reply: &DurableReplyProjection| reply.message_id.clone());
    let attachments = if deleted {
        Vec::new()
    } else {
        sqlx::query(
            "SELECT id,original_filename,content_type,file_size,state_version \
             FROM attachments WHERE message_id=? AND media_state='attached' ORDER BY id",
        )
        .bind(message_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|attachment| DurableAttachmentProjection {
            attachment_id: attachment.get(0),
            filename: attachment.get(1),
            content_type: attachment.get(2),
            file_size: attachment.get(3),
            state_version: attachment.get::<i64, _>(4) as u64,
        })
        .collect()
    };
    let mentions = if deleted {
        Vec::new()
    } else {
        sqlx::query(
            "SELECT mention_kind,target_id,start_byte,end_byte FROM message_mentions \
             WHERE message_id=? ORDER BY ordinal",
        )
        .bind(message_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|mention| {
            let kind = match mention.get::<&str, _>(0) {
                "user" => crate::engine::messaging::MentionKind::User,
                "role" => crate::engine::messaging::MentionKind::Role,
                _ => crate::engine::messaging::MentionKind::Everyone,
            };
            crate::engine::messaging::MessageMention {
                kind,
                target_id: mention.get(1),
                start_byte: mention.get::<i64, _>(2) as usize,
                end_byte: mention.get::<i64, _>(3) as usize,
            }
        })
        .collect()
    };
    let rich_embeds = if deleted {
        None
    } else {
        row.get::<Option<&str>, _>(12)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| ReplayError::DependencyUnavailable)?
    };
    let components = if deleted {
        None
    } else {
        row.get::<Option<&str>, _>(13)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| ReplayError::DependencyUnavailable)?
    };
    Ok(Some(DurableMessageProjection {
        message_id: row.get(0),
        conversation_id: ConversationId::from_stored(row.get::<String, _>(1))
            .map_err(|_| ReplayError::InvalidInput)?,
        sequence: row.get::<i64, _>(2).to_string(),
        entity_version: row.get::<i64, _>(3) as u64,
        sender_id: row.get(4),
        sender_nick: row.get(5),
        content: (!deleted).then(|| row.get(6)),
        content_format: row.get(7),
        created_at: row.get(8),
        edited_at: row.get(9),
        deleted,
        reply_to_id,
        reply_to,
        attachments,
        mentions,
        rich_embeds,
        components,
    }))
}
