use super::{
    ConversationId, DurableMessageProjection, ReplayError, SqliteConnection,
    load_message_projection,
};

pub(super) async fn load_snapshot_messages(
    connection: &mut SqliteConnection,
    subscriptions: &[ConversationId],
    message_limit: usize,
) -> Result<Vec<DurableMessageProjection>, ReplayError> {
    if subscriptions.is_empty() {
        return Ok(Vec::new());
    }
    let mut per_conversation = Vec::with_capacity(subscriptions.len());
    for subscription in subscriptions {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM messages WHERE conversation_id=? \
             ORDER BY conversation_sequence DESC LIMIT ?",
        )
        .bind(subscription.as_str())
        .bind(message_limit as i64)
        .fetch_all(&mut *connection)
        .await?;
        per_conversation.push(ids);
    }
    let mut ids = Vec::with_capacity(message_limit);
    for position in 0..message_limit {
        for conversation in &per_conversation {
            if let Some(id) = conversation.get(position) {
                ids.push(id.clone());
                if ids.len() == message_limit {
                    break;
                }
            }
        }
        if ids.len() == message_limit {
            break;
        }
    }
    let mut messages = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(message) = load_message_projection(connection, &id).await? {
            messages.push(message);
        }
    }
    messages.sort_by(|left, right| {
        left.conversation_id
            .cmp(&right.conversation_id)
            .then_with(|| {
                left.sequence
                    .parse::<u64>()
                    .unwrap_or_default()
                    .cmp(&right.sequence.parse::<u64>().unwrap_or_default())
            })
    });
    Ok(messages)
}

pub(super) async fn load_history_boundaries(
    connection: &mut SqliteConnection,
    messages: &[DurableMessageProjection],
) -> Result<std::collections::BTreeMap<String, String>, ReplayError> {
    let mut earliest = std::collections::BTreeMap::<ConversationId, (&str, i64)>::new();
    for message in messages {
        let sequence = message.sequence.parse::<i64>().unwrap_or_default();
        earliest
            .entry(message.conversation_id.clone())
            .and_modify(|entry| {
                if sequence < entry.1 {
                    *entry = (&message.created_at, sequence);
                }
            })
            .or_insert((&message.created_at, sequence));
    }
    let mut boundaries = std::collections::BTreeMap::new();
    for (conversation_id, (created_at, sequence)) in earliest {
        let older: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages \
             WHERE conversation_id=? AND conversation_sequence<?)",
        )
        .bind(conversation_id.as_str())
        .bind(sequence)
        .fetch_one(&mut *connection)
        .await?;
        if older {
            boundaries.insert(conversation_id.into_inner(), created_at.to_owned());
        }
    }
    Ok(boundaries)
}
