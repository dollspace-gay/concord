use super::{ChatEngine, ConnectionId, Synchronization, normalize_channel_name};

impl ChatEngine {
    pub async fn synchronize(
        &self,
        session_id: ConnectionId,
        subscriptions: &[String],
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Synchronization, crate::engine::replay::ReplayError> {
        let actor = self
            .authenticated_actors
            .get(&session_id)
            .map(|entry| entry.clone())
            .ok_or(crate::engine::replay::ReplayError::Unavailable)?;
        if let Some(cursor) = cursor {
            self.replay
                .get()
                .ok_or(crate::engine::replay::ReplayError::Unavailable)?
                .replay(&actor, subscriptions, cursor, limit)
                .await
                .map(Synchronization::Replay)
        } else {
            self.replay
                .get()
                .ok_or(crate::engine::replay::ReplayError::Unavailable)?
                .snapshot_with_limit(&actor, subscriptions, limit)
                .await
                .map(Synchronization::Snapshot)
        }
    }
    pub async fn conversation_id_for_channel(
        &self,
        server_id: &str,
        channel: &str,
    ) -> Result<String, String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        sqlx::query_scalar(
            "SELECT cv.id FROM conversations cv JOIN channels c ON c.id=cv.channel_id \
             WHERE cv.kind='channel' AND c.server_id=? AND c.name=?",
        )
        .bind(server_id)
        .bind(normalize_channel_name(channel))
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("DB error: {error}"))?
        .ok_or_else(|| "resource unavailable".into())
    }
    pub fn bind_authenticated_actor(
        &self,
        session_id: ConnectionId,
        actor: crate::auth::authority::Actor,
    ) -> Result<(), String> {
        let session = self.sessions.get(&session_id).ok_or("Session not found")?;
        if session.user_id.as_deref() != Some(actor.user_id().as_str()) {
            return Err("authenticated actor does not match connection identity".into());
        }
        self.credential_connections
            .entry(actor.credential_id().clone())
            .or_default()
            .insert(session_id);
        self.authenticated_actors.insert(session_id, actor);
        Ok(())
    }
    pub fn get_authenticated_actor(
        &self,
        session_id: ConnectionId,
    ) -> Option<crate::auth::authority::Actor> {
        self.authenticated_actors
            .get(&session_id)
            .map(|actor| actor.clone())
    }
    /// Get the configured maximum message length.
    pub fn max_message_length(&self) -> usize {
        self.max_message_length
    }
    /// Get the configured maximum file upload size in megabytes.
    pub fn max_file_size_mb(&self) -> u64 {
        self.max_file_size_mb
    }
    /// Remove stale rate-limiter buckets that haven't been used recently.
    pub fn cleanup_rate_limiter(&self) {
        self.message_limiter
            .cleanup(std::time::Duration::from_secs(600));
    }
    /// Remove stale slow mode cache entries older than the given duration.
    pub fn cleanup_slowmode_cache(&self) {
        let cutoff = std::time::Duration::from_secs(600);
        self.slowmode_last_sent
            .retain(|_, instant| instant.elapsed() < cutoff);
    }
}
