use super::{
    ConversationId, DurableMessageProjection, DurableReactionProjection, DurableReadProjection,
    MAX_SNAPSHOT_REACTION_GROUPS, ReplayError, Row, SnapshotReactionGroup, SqliteConnection,
};

pub(super) async fn load_snapshot_reactions(
    connection: &mut SqliteConnection,
    messages: &[DurableMessageProjection],
    principal_id: &str,
) -> Result<Vec<SnapshotReactionGroup>, ReplayError> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder =
        sqlx::QueryBuilder::new("SELECT r.message_id,r.emoji,COUNT(*),MAX(CASE WHEN r.user_id=");
    builder.push_bind(principal_id);
    builder.push(
        " THEN 1 ELSE 0 END) \
         FROM reactions r JOIN messages m ON m.id=r.message_id \
         WHERE m.deleted_at IS NULL AND r.message_id IN (",
    );
    let mut separated = builder.separated(",");
    for message in messages {
        separated.push_bind(&message.message_id);
    }
    separated
        .push_unseparated(") GROUP BY r.message_id,r.emoji ORDER BY r.message_id,r.emoji LIMIT ");
    builder.push_bind((MAX_SNAPSHOT_REACTION_GROUPS + 1) as i64);
    let rows = builder.build().fetch_all(&mut *connection).await?;
    if rows.len() > MAX_SNAPSHOT_REACTION_GROUPS {
        return Err(ReplayError::SnapshotTooLarge);
    }
    let mut reactions = Vec::with_capacity(rows.len());
    for row in rows {
        reactions.push(SnapshotReactionGroup {
            message_id: row.get(0),
            emoji: row.get(1),
            count: row.get::<i64, _>(2) as u64,
            reacted_by_me: row.get::<i64, _>(3) != 0,
        });
    }
    Ok(reactions)
}

pub(super) async fn load_snapshot_reads(
    connection: &mut SqliteConnection,
    principal_id: &str,
    subscriptions: &[ConversationId],
) -> Result<Vec<DurableReadProjection>, ReplayError> {
    if subscriptions.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT m.conversation_id,rs.last_read_message_id,rs.conversation_sequence \
         FROM read_states rs JOIN messages m ON m.id=rs.last_read_message_id \
         WHERE rs.user_id=",
    );
    builder.push_bind(principal_id);
    builder.push(" AND m.conversation_id IN (");
    let mut separated = builder.separated(",");
    for subscription in subscriptions {
        separated.push_bind(subscription.as_str());
    }
    separated.push_unseparated(") ORDER BY m.conversation_id");
    let rows = builder.build().fetch_all(&mut *connection).await?;
    let mut reads = Vec::with_capacity(rows.len());
    for row in rows {
        let conversation_id = ConversationId::from_stored(row.get::<String, _>(0))
            .map_err(|_| ReplayError::InvalidInput)?;
        let entity_id =
            crate::engine::messaging::read_entity_id(principal_id, conversation_id.as_str());
        let version = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM entity_versions WHERE entity_type='read_state' AND entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&mut *connection)
        .await?
        .unwrap_or(1);
        reads.push(DurableReadProjection {
            conversation_id,
            message_id: row.get(1),
            sequence: row.get::<i64, _>(2).to_string(),
            entity_version: version as u64,
        });
    }
    Ok(reads)
}

pub(super) async fn load_reaction_projection(
    connection: &mut SqliteConnection,
    entity_id: &str,
    descriptor: &serde_json::Value,
) -> Result<Option<DurableReactionProjection>, ReplayError> {
    let field = |name| {
        descriptor
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or(ReplayError::InvalidInput)
    };
    let message_id = field("message_id")?;
    let user_id = field("user_id")?;
    let emoji = field("emoji")?;
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM reactions r JOIN messages m ON m.id=r.message_id \
         WHERE r.message_id=? AND r.user_id=? AND r.emoji=? AND m.deleted_at IS NULL)",
    )
    .bind(message_id)
    .bind(user_id)
    .bind(emoji)
    .fetch_one(&mut *connection)
    .await?;
    let version = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM entity_versions WHERE entity_type='reaction' AND entity_id=?",
    )
    .bind(entity_id)
    .fetch_optional(&mut *connection)
    .await?
    .unwrap_or(1);
    Ok(Some(DurableReactionProjection {
        message_id: message_id.to_owned(),
        user_id: user_id.to_owned(),
        emoji: emoji.to_owned(),
        present,
        entity_version: version as u64,
    }))
}

pub(super) async fn load_read_projection(
    connection: &mut SqliteConnection,
    principal_id: &str,
    conversation_id: &str,
    entity_id: &str,
) -> Result<Option<DurableReadProjection>, ReplayError> {
    let row = sqlx::query(
        "SELECT rs.last_read_message_id,rs.conversation_sequence,COALESCE(ev.version,1) \
         FROM read_states rs JOIN messages m ON m.id=rs.last_read_message_id \
         LEFT JOIN entity_versions ev ON ev.entity_type='read_state' AND ev.entity_id=? \
         WHERE rs.user_id=? AND m.conversation_id=?",
    )
    .bind(entity_id)
    .bind(principal_id)
    .bind(conversation_id)
    .fetch_optional(connection)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let conversation_id =
        ConversationId::from_stored(conversation_id).map_err(|_| ReplayError::InvalidInput)?;
    Ok(Some(DurableReadProjection {
        conversation_id,
        message_id: row.get(0),
        sequence: row.get::<i64, _>(1).to_string(),
        entity_version: row.get::<i64, _>(2) as u64,
    }))
}
