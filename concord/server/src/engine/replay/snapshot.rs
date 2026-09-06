use super::{
    Actor, MAX_SNAPSHOT_MESSAGES, MAX_SNAPSHOT_PROJECTION_BYTES, PROTOCOL_VERSION, ReplayError,
    ReplayService, SyncSnapshot, authorize_conversation, canonical_subscriptions,
    load_history_boundaries, load_snapshot_messages, load_snapshot_reactions, load_snapshot_reads,
    map_auth_error,
};

impl ReplayService {
    pub async fn snapshot(
        &self,
        actor: &Actor,
        subscriptions: &[String],
    ) -> Result<SyncSnapshot, ReplayError> {
        self.snapshot_with_limit(actor, subscriptions, MAX_SNAPSHOT_MESSAGES)
            .await
    }

    pub async fn snapshot_with_limit(
        &self,
        actor: &Actor,
        subscriptions: &[String],
        message_limit: usize,
    ) -> Result<SyncSnapshot, ReplayError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::Replay);
        let subscriptions = canonical_subscriptions(subscriptions)?;
        let message_limit = message_limit.clamp(1, MAX_SNAPSHOT_MESSAGES);
        self.auth
            .validate_actor(actor)
            .await
            .map_err(map_auth_error)?;
        let operation_generation = self
            .write_admission
            .current_operation_generation()
            .await
            .map_err(|_| ReplayError::Unavailable)?;
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        for conversation_id in &subscriptions {
            authorize_conversation(
                &self.authorization,
                &self.auth,
                &mut transaction,
                actor,
                conversation_id.as_str(),
            )
            .await?;
        }
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let high_water: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(event_sequence),0) FROM event_log")
                .fetch_one(&mut *transaction)
                .await?;
        let read_states =
            load_snapshot_reads(&mut transaction, actor.user_id().as_str(), &subscriptions).await?;
        let mut effective_limit = message_limit;
        let (messages, reactions) = loop {
            let messages =
                load_snapshot_messages(&mut transaction, &subscriptions, effective_limit).await?;
            let reactions = match load_snapshot_reactions(
                &mut transaction,
                &messages,
                actor.user_id().as_str(),
            )
            .await
            {
                Ok(reactions) => reactions,
                Err(ReplayError::SnapshotTooLarge) if effective_limit > 1 => {
                    effective_limit = (effective_limit / 2).max(1);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let projected_bytes = serde_json::to_vec(&(&messages, &reactions, &read_states))
                .map_err(|_| ReplayError::InvalidInput)?
                .len();
            if projected_bytes <= MAX_SNAPSHOT_PROJECTION_BYTES {
                break (messages, reactions);
            }
            if effective_limit == 1 {
                return Err(ReplayError::SnapshotTooLarge);
            }
            effective_limit = (effective_limit / 2).max(1);
        };
        let history_before = load_history_boundaries(&mut transaction, &messages).await?;
        transaction.commit().await?;
        let cursor = self.encode_cursor(actor, &subscriptions, &generation, high_water)?;
        let snapshot = SyncSnapshot {
            protocol_version: PROTOCOL_VERSION,
            operation_generation,
            cursor,
            messages,
            reactions,
            read_states,
            history_before,
        };
        metric.succeed();
        Ok(snapshot)
    }
}
