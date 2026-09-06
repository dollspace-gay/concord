use super::{Arc, ChatEngine, Row};

impl ChatEngine {
    /// Poll the transactional outbox. Wakeups reduce latency; polling guarantees
    /// recovery after process restart or a missed bounded hint.
    pub async fn run_delivery_dispatcher(
        self: Arc<Self>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        let messaging = self
            .messaging
            .get()
            .ok_or_else(|| "messaging service unavailable".to_string())?;
        let mut wakeups = messaging.subscribe_wakeups();
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(500));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut maintenance_ticks = 0_u32;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = poll.tick() => {},
                _ = wakeups.recv() => {},
            }
            self.process_moderation_cleanup_batch().await?;
            loop {
                if shutdown.is_cancelled() || self.dispatch_outbox_batch().await? < 100 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            maintenance_ticks = maintenance_ticks.wrapping_add(1);
            if maintenance_ticks.is_multiple_of(120) {
                self.prune_delivery_retention().await?;
                self.archive_due_threads().await?;
            }
        }
    }
    pub(super) async fn archive_due_threads(&self) -> Result<(), String> {
        let writes = self
            .write_admission
            .as_ref()
            .ok_or_else(|| "thread write admission unavailable".to_string())?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        let thread_versions =
            crate::db::queries::threads::archive_due_threads(&mut transaction, 100)
                .await
                .map_err(|error| error.to_string())?;
        for (thread_id, version) in &thread_versions {
            Self::insert_thread_state_event_in(
                &mut transaction,
                thread_id,
                *version,
                true,
                Some("inactivity"),
                "system",
            )
            .await?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        for (thread_id, _) in thread_versions {
            self.project_thread_state(&thread_id).await?;
        }
        Ok(())
    }
    /// Advances at most one ban-cleanup job and at most one hundred messages.
    /// The job row and every canonical tombstone/event are committed together,
    /// so restart resumes from the remaining undeleted rows.
    pub(super) async fn process_moderation_cleanup_batch(&self) -> Result<usize, String> {
        let writes = self
            .write_admission
            .as_ref()
            .ok_or_else(|| "moderation cleanup write admission unavailable".to_string())?;
        let (_permit, mut transaction) =
            writes.begin().await.map_err(|_| "resource unavailable")?;
        let job = sqlx::query(
            "SELECT id,server_id,user_id,actor_id,cutoff_at \
             FROM moderation_cleanup_jobs WHERE state='pending' \
             ORDER BY created_at,id LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        let Some(job) = job else {
            transaction
                .commit()
                .await
                .map_err(|_| "resource unavailable")?;
            return Ok(0);
        };
        let job_id: String = job.get(0);
        let server_id: String = job.get(1);
        let user_id: String = job.get(2);
        let actor_id: String = job.get(3);
        let cutoff_at: String = job.get(4);
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| "resource unavailable")?;
        let messages = sqlx::query(
            "SELECT m.id FROM messages m \
             JOIN channels c ON c.id=m.channel_id AND c.server_id=m.server_id \
             JOIN moderation_cleanup_scopes s ON s.job_id=? \
                AND s.conversation_id=m.conversation_id \
                AND m.conversation_sequence<=s.through_sequence \
             WHERE m.server_id=? AND m.sender_id=? AND m.deleted_at IS NULL \
               AND julianday(m.created_at)>=julianday(?) \
             ORDER BY m.created_at,m.id LIMIT 100",
        )
        .bind(&job_id)
        .bind(&server_id)
        .bind(&user_id)
        .bind(&cutoff_at)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        for message in &messages {
            let message_id: String = message.get(0);
            crate::engine::messaging::tombstone_moderated_message_in(
                &mut transaction,
                &generation,
                &message_id,
                &actor_id,
            )
            .await
            .map_err(|_| "resource unavailable")?
            .ok_or_else(|| "resource unavailable".to_string())?;
        }
        let remaining: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages m \
             JOIN channels c ON c.id=m.channel_id AND c.server_id=m.server_id \
             JOIN moderation_cleanup_scopes s ON s.job_id=? \
                AND s.conversation_id=m.conversation_id \
                AND m.conversation_sequence<=s.through_sequence \
             WHERE m.server_id=? AND m.sender_id=? AND m.deleted_at IS NULL \
               AND julianday(m.created_at)>=julianday(?))",
        )
        .bind(&job_id)
        .bind(&server_id)
        .bind(&user_id)
        .bind(&cutoff_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        sqlx::query(
            "UPDATE moderation_cleanup_jobs SET deleted_count=deleted_count+?, \
             state=?,updated_at=datetime('now') WHERE id=?",
        )
        .bind(messages.len() as i64)
        .bind(if remaining { "pending" } else { "completed" })
        .bind(&job_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable")?;
        Ok(messages.len())
    }
}
