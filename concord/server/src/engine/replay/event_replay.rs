use super::{
    Actor, ConversationId, DurableEventProjection, MAX_REPLAY_EVENTS, PROTOCOL_VERSION,
    ReplayBatch, ReplayError, ReplayService, ResyncReason, Row, authorize_conversation,
    canonical_subscriptions, load_message_projection, load_reaction_projection,
    load_read_projection, map_auth_error, resolve_current_event_state,
};

impl ReplayService {
    pub async fn replay(
        &self,
        actor: &Actor,
        subscriptions: &[String],
        cursor: &str,
        limit: usize,
    ) -> Result<ReplayBatch, ReplayError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Replay);
        let subscriptions = canonical_subscriptions(subscriptions)?;
        let claims = self.decode_cursor(cursor)?;
        self.validate_cursor_actor(&claims, actor, &subscriptions)?;
        self.auth
            .validate_actor(actor)
            .await
            .map_err(map_auth_error)?;
        let operation_generation = self
            .write_admission
            .current_operation_generation()
            .await
            .map_err(|_| ReplayError::Unavailable)?;
        let limit = limit.clamp(1, MAX_REPLAY_EVENTS);
        let mut transaction = self.pool.begin().await?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        if generation != claims.database_generation {
            return Err(ReplayError::ResyncRequired(ResyncReason::DatabaseRestored));
        }
        for conversation_id in &subscriptions {
            match authorize_conversation(
                &self.authorization,
                &self.auth,
                &mut transaction,
                actor,
                conversation_id.as_str(),
            )
            .await
            {
                Ok(()) => {}
                Err(ReplayError::Unavailable) => {
                    return Err(ReplayError::ResyncRequired(ResyncReason::AccessRevoked));
                }
                Err(error) => return Err(error),
            }
        }
        let retained_from: i64 = sqlx::query_scalar(
            "SELECT retained_from_sequence FROM event_retention_state WHERE singleton=1",
        )
        .fetch_one(&mut *transaction)
        .await?;
        // A cursor stores the last consumed sequence. It is replayable when the next
        // sequence is still retained, including retained_from - 1 at the boundary.
        if claims.event_sequence.saturating_add(1) < retained_from {
            return Err(ReplayError::ResyncRequired(ResyncReason::CursorExpired));
        }
        let global_high_water: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(event_sequence),0) FROM event_log")
                .fetch_one(&mut *transaction)
                .await?;
        let rows = if subscriptions.is_empty() {
            Vec::new()
        } else {
            let mut builder = sqlx::QueryBuilder::new(
                "SELECT event_sequence,conversation_id,event_kind,entity_type,entity_id, \
                        entity_version,descriptor_json FROM event_log \
                 WHERE database_generation=",
            );
            builder.push_bind(&generation);
            builder.push(" AND event_sequence>");
            builder.push_bind(claims.event_sequence);
            builder.push(" AND conversation_id IN (");
            let mut separated = builder.separated(",");
            for subscription in &subscriptions {
                separated.push_bind(subscription.as_str());
            }
            separated.push_unseparated(
                ") AND (entity_type<>'read_state' OR json_extract(descriptor_json,'$.user_id')=",
            );
            builder.push_bind(actor.user_id().as_str());
            builder.push(") ORDER BY event_sequence LIMIT ");
            builder.push_bind((limit + 1) as i64);
            builder.build().fetch_all(&mut *transaction).await?
        };
        let mut scanned_high_water = claims.event_sequence;
        let mut events = Vec::new();
        let has_more = rows.len() > limit;
        for row in rows.into_iter().take(limit) {
            let event_sequence: i64 = row.get(0);
            scanned_high_water = event_sequence;
            let Some(conversation_id) = row.get::<Option<String>, _>(1) else {
                continue;
            };
            let event_kind: String = row.get(2);
            let entity_type: String = row.get(3);
            let entity_id: String = row.get(4);
            let entity_version: i64 = row.get(5);
            let mut descriptor: serde_json::Value = serde_json::from_str(row.get::<&str, _>(6))
                .map_err(|_| ReplayError::InvalidInput)?;
            if entity_type == "read_state"
                && descriptor
                    .get("user_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(actor.user_id().as_str())
            {
                continue;
            }
            let message = if entity_type == "message" {
                load_message_projection(&mut transaction, &entity_id).await?
            } else {
                None
            };
            let reaction = if entity_type == "reaction" {
                load_reaction_projection(&mut transaction, &entity_id, &descriptor).await?
            } else {
                None
            };
            let read_state = if entity_type == "read_state" {
                load_read_projection(
                    &mut transaction,
                    actor.user_id().as_str(),
                    &conversation_id,
                    &entity_id,
                )
                .await?
            } else {
                None
            };
            let current_entity_version = resolve_current_event_state(
                &mut transaction,
                &entity_type,
                &entity_id,
                entity_version,
                &mut descriptor,
            )
            .await?;
            let conversation_id = ConversationId::from_stored(conversation_id)
                .map_err(|_| ReplayError::InvalidInput)?;
            events.push(DurableEventProjection {
                kind: event_kind,
                conversation_id,
                entity_type,
                entity_id,
                entity_version: current_entity_version as u64,
                message,
                reaction,
                read_state,
                descriptor,
            });
        }
        if !has_more {
            scanned_high_water = global_high_water;
        }
        transaction.commit().await?;
        let next_cursor =
            self.encode_cursor(actor, &subscriptions, &generation, scanned_high_water)?;
        let batch = ReplayBatch {
            protocol_version: PROTOCOL_VERSION,
            operation_generation,
            cursor: next_cursor,
            events,
            has_more,
        };
        metric.succeed();
        Ok(batch)
    }
}
