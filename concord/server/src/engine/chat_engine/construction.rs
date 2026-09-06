use super::{ChatEngine, DashMap, OnceLock, RateLimiter, SqlitePool};

impl ChatEngine {
    pub(crate) async fn begin_admitted_write(
        &self,
    ) -> Result<
        (
            tokio::sync::OwnedSemaphorePermit,
            sqlx::Transaction<'static, sqlx::Sqlite>,
        ),
        String,
    > {
        self.write_admission
            .as_ref()
            .ok_or_else(|| "DEPENDENCY_UNAVAILABLE: write dependency unavailable".to_string())?
            .begin()
            .await
            .map_err(|_| "DEPENDENCY_UNAVAILABLE: write dependency unavailable".to_string())
    }
    pub fn new(
        db: SqlitePool,
        auth: crate::auth::authority::AuthService,
        replay_secret: &str,
        max_message_length: usize,
        max_file_size_mb: u64,
    ) -> Self {
        let write_admission = crate::engine::write_admission::WriteAdmission::new(db.clone());
        let messaging = crate::engine::messaging::MessagingService::new_with_write_admission(
            db.clone(),
            auth.clone(),
            max_message_length,
            write_admission.clone(),
        );
        let replay = crate::engine::replay::ReplayService::new_with_write_admission(
            db.clone(),
            auth.clone(),
            replay_secret,
            write_admission.clone(),
        );
        Self {
            sessions: DashMap::new(),
            user_connections: DashMap::new(),
            servers: DashMap::new(),
            channels: DashMap::new(),
            channel_name_index: DashMap::new(),
            server_alias_index: DashMap::new(),
            server_aliases: DashMap::new(),
            nick_to_session: DashMap::new(),
            authenticated_actors: DashMap::new(),
            credential_connections: DashMap::new(),
            db: Some(db),
            auth: OnceLock::from(auth),
            messaging: OnceLock::from(messaging),
            replay: OnceLock::from(replay),
            search_token_secret: replay_secret.to_owned(),
            integration_vault: OnceLock::new(),
            write_admission: Some(write_admission),
            message_limiter: RateLimiter::new(10, 1.0),
            max_message_length,
            max_file_size_mb,
            slowmode_last_sent: DashMap::new(),
        }
    }
    #[cfg(test)]
    pub(crate) fn test_harness(max_message_length: usize, max_file_size_mb: u64) -> Self {
        Self {
            sessions: DashMap::new(),
            user_connections: DashMap::new(),
            servers: DashMap::new(),
            channels: DashMap::new(),
            channel_name_index: DashMap::new(),
            server_alias_index: DashMap::new(),
            server_aliases: DashMap::new(),
            nick_to_session: DashMap::new(),
            authenticated_actors: DashMap::new(),
            credential_connections: DashMap::new(),
            db: None,
            auth: OnceLock::new(),
            messaging: OnceLock::new(),
            replay: OnceLock::new(),
            search_token_secret: "test-search-token-secret".into(),
            integration_vault: OnceLock::new(),
            write_admission: None,
            message_limiter: RateLimiter::new(10, 1.0),
            max_message_length,
            max_file_size_mb,
            slowmode_last_sent: DashMap::new(),
        }
    }
    pub fn replay_service(&self) -> &crate::engine::replay::ReplayService {
        self.replay
            .get()
            .expect("production constructor installs replay service")
    }
}
