use super::{ChatEngine, ChatEvent, Row};

impl ChatEngine {
    pub(super) async fn prune_delivery_retention(&self) -> Result<usize, String> {
        let pool = self
            .db
            .as_ref()
            .ok_or_else(|| "delivery database unavailable".to_string())?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| error.to_string())?;
        sqlx::query(
            "DELETE FROM command_receipts WHERE rowid IN ( \
                 SELECT cr.rowid FROM command_receipts cr \
                 JOIN operation_generations g ON g.generation=cr.operation_generation \
                 WHERE g.expires_at<=unixepoch() \
                   AND (cr.canonical_message_id IS NULL OR NOT EXISTS( \
                       SELECT 1 FROM messages m WHERE m.id=cr.canonical_message_id \
                   )) \
                 ORDER BY cr.rowid LIMIT 500 \
             )",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        let candidates: Vec<i64> = sqlx::query_scalar(
            "SELECT e.event_sequence FROM event_log e \
             JOIN delivery_outbox o ON o.event_sequence=e.event_sequence \
             JOIN event_retention_state r ON r.singleton=1 \
             WHERE o.completed_at IS NOT NULL \
               AND e.event_sequence<=r.dispatcher_high_water \
               AND e.created_at<datetime('now','-' || r.retention_seconds || ' seconds') \
             ORDER BY e.event_sequence LIMIT 500",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        for event_sequence in &candidates {
            sqlx::query(
                "DELETE FROM delivery_outbox WHERE event_sequence=? AND completed_at IS NOT NULL",
            )
            .bind(event_sequence)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM event_log WHERE event_sequence=?")
                .bind(event_sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
        }
        sqlx::query(
            "UPDATE event_retention_state SET retained_from_sequence=COALESCE( \
                 (SELECT MIN(event_sequence) FROM event_log), \
                 (SELECT seq+1 FROM sqlite_sequence WHERE name='event_log'),0 \
             ),updated_at=datetime('now') WHERE singleton=1",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        Ok(candidates.len())
    }
    pub(super) async fn dispatch_outbox_batch(&self) -> Result<usize, String> {
        let pool = self
            .db
            .as_ref()
            .ok_or_else(|| "delivery database unavailable".to_string())?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| error.to_string())?;
        let mut event_sequences: Vec<i64> = sqlx::query_scalar(
            "UPDATE delivery_outbox SET attempts=attempts+1, \
                    claimed_until=datetime('now','+30 seconds'),last_error=NULL \
             WHERE event_sequence IN ( \
                 SELECT event_sequence FROM delivery_outbox \
                 WHERE completed_at IS NULL AND available_at<=datetime('now') \
                   AND (claimed_until IS NULL OR claimed_until<=datetime('now')) \
                 ORDER BY event_sequence LIMIT 100 \
             ) RETURNING event_sequence",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        event_sequences.sort_unstable();

        for event_sequence in &event_sequences {
            let target = sqlx::query(
                "SELECT c.kind,c.channel_id,c.id FROM event_log e \
                 JOIN conversations c ON c.id=e.conversation_id WHERE e.event_sequence=?",
            )
            .bind(event_sequence)
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?;
            let mut session_ids = std::collections::HashSet::new();
            if let Some(target) = target {
                let kind: String = target.get(0);
                if kind == "channel" {
                    if let Some(channel_id) = target.get::<Option<String>, _>(1)
                        && let Some(channel) = self.channels.get(&channel_id)
                    {
                        session_ids.extend(channel.members.iter().copied());
                    }
                } else {
                    let conversation_id: String = target.get(2);
                    let participants: Vec<String> = sqlx::query_scalar(
                        "SELECT user_id FROM conversation_participants \
                         WHERE conversation_id=? AND left_at IS NULL",
                    )
                    .bind(conversation_id)
                    .fetch_all(pool)
                    .await
                    .map_err(|error| error.to_string())?;
                    for participant in participants {
                        if let Some(connections) = self.user_connections.get(&participant) {
                            session_ids.extend(connections.iter().copied());
                        }
                    }
                }
            }
            let mut failed = None;
            for session_id in session_ids {
                let Some(actor) = self
                    .authenticated_actors
                    .get(&session_id)
                    .map(|entry| entry.clone())
                else {
                    continue;
                };
                match self
                    .replay_service()
                    .project_event(&actor, *event_sequence)
                    .await
                {
                    Ok(Some((conversation_id, event))) => {
                        if let Some(session) = self.sessions.get(&session_id) {
                            session.send_guarded(
                                ChatEvent::DurableEvent {
                                    event: Box::new(event),
                                },
                                Some(crate::engine::user_session::DeliveryGuard::Conversations(
                                    vec![conversation_id.into_inner()],
                                )),
                            );
                        }
                    }
                    Ok(None)
                    | Err(crate::engine::replay::ReplayError::ResyncRequired(_))
                    | Err(crate::engine::replay::ReplayError::Unavailable) => {}
                    Err(error) => {
                        failed = Some(error.to_string());
                        break;
                    }
                }
            }
            if let Some(error) = failed {
                sqlx::query(
                    "UPDATE delivery_outbox SET claimed_until=NULL,last_error=?, \
                            available_at=datetime('now','+1 second') WHERE event_sequence=?",
                )
                .bind(error)
                .bind(event_sequence)
                .execute(pool)
                .await
                .map_err(|error| error.to_string())?;
            } else {
                let mut transaction = pool
                    .begin_with("BEGIN IMMEDIATE")
                    .await
                    .map_err(|error| error.to_string())?;
                sqlx::query(
                    "UPDATE delivery_outbox SET completed_at=datetime('now'),claimed_until=NULL \
                     WHERE event_sequence=?",
                )
                .bind(event_sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
                sqlx::query(
                    "UPDATE event_retention_state SET dispatcher_high_water=( \
                         SELECT COALESCE(MIN(event_sequence)-1, \
                             (SELECT COALESCE(MAX(event_sequence),0) FROM event_log)) \
                         FROM delivery_outbox WHERE completed_at IS NULL \
                     ),updated_at=datetime('now') WHERE singleton=1",
                )
                .execute(&mut *transaction)
                .await
                .map_err(|error| error.to_string())?;
                transaction
                    .commit()
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(event_sequences.len())
    }
}
