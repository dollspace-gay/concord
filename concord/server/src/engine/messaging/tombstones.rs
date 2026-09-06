use super::{
    EventIdentity, MessageTarget, MessagingError, Row, SqliteConnection, insert_event,
    propagate_announcement_delete, set_entity_version,
};

pub(super) async fn tombstone_message_in(
    connection: &mut SqliteConnection,
    generation: &str,
    target: &MessageTarget,
    actor_id: &str,
) -> Result<(i64, i64), MessagingError> {
    let version: i64 = sqlx::query_scalar(
        "UPDATE messages SET deleted_at=datetime('now'),entity_version=entity_version+1 \
         WHERE id=? AND deleted_at IS NULL RETURNING entity_version",
    )
    .bind(&target.message_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(MessagingError::Unavailable)?;
    sqlx::query(
        "UPDATE attachments SET media_state='deleting',state_version=state_version+1, \
         delete_after=datetime('now','+1 hour') \
         WHERE message_id=? AND media_state='attached'",
    )
    .bind(&target.message_id)
    .execute(&mut *connection)
    .await?;
    set_entity_version(connection, "message", &target.message_id, version).await?;
    crate::db::queries::atproto::schedule_source_mutation(
        connection,
        &target.message_id,
        version,
        true,
    )
    .await?;
    let event_sequence = insert_event(
        connection,
        generation,
        target,
        EventIdentity {
            kind: "message_deleted",
            entity_type: "message",
            entity_id: &target.message_id,
            version,
        },
        actor_id,
        serde_json::json!({"message_id": target.message_id}),
    )
    .await?;
    propagate_announcement_delete(
        connection,
        generation,
        &target.message_id,
        version,
        actor_id,
    )
    .await?;
    Ok((version, event_sequence))
}

/// Canonical deletion primitive for an already-authorized durable moderation
/// job. Returning `None` lets a resumed batch skip a concurrently completed
/// tombstone without inventing a second version or event.
pub(crate) async fn tombstone_moderated_message_in(
    connection: &mut SqliteConnection,
    generation: &str,
    message_id: &str,
    actor_id: &str,
) -> Result<Option<i64>, MessagingError> {
    let row = sqlx::query(
        "SELECT m.id,m.conversation_id,m.conversation_sequence,m.server_id,m.channel_id, \
                m.sender_id,c.authorization_version,m.deleted_at \
         FROM messages m JOIN channels c ON c.id=m.channel_id AND c.server_id=m.server_id \
         WHERE m.id=? AND m.deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let target = MessageTarget {
        message_id: row.get(0),
        conversation_id: row.get(1),
        conversation_sequence: row.get(2),
        server_id: row.get(3),
        channel_id: row.get(4),
        sender_id: row.get(5),
        authorization_version: row.get(6),
        direct: false,
        deleted: row.get::<Option<String>, _>(7).is_some(),
    };
    let (_, event_sequence) =
        tombstone_message_in(connection, generation, &target, actor_id).await?;
    Ok(Some(event_sequence))
}
