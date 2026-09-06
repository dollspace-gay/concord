use super::{Arc, ChatEngine, ConnectionId, SqlitePool, UserSession, moderation_dependency};

impl ChatEngine {
    /// Get a reference to the database pool (if configured).
    pub fn db(&self) -> Option<&SqlitePool> {
        self.db.as_ref()
    }
    /// Check if a nickname is available.
    pub fn is_nick_available(&self, nickname: &str) -> bool {
        !self
            .nick_to_session
            .contains_key(&crate::auth::authority::rfc1459_casefold(nickname))
    }
    /// Look up a session ID by nickname. Returns None if no session with that nick exists.
    pub fn get_session_id_by_nick(&self, nickname: &str) -> Option<ConnectionId> {
        self.nick_to_session
            .get(&crate::auth::authority::rfc1459_casefold(nickname))
            .map(|r| *r)
    }
    /// Get the user_id for a session. Returns None if session not found or has no user.
    pub fn get_session_user_id(&self, session_id: ConnectionId) -> Option<String> {
        self.sessions
            .get(&session_id)
            .and_then(|s| s.user_id.clone())
    }
    /// Get (server_id, channel_name) pairs for all channels a session is in.
    pub fn get_session_channels(&self, session_id: ConnectionId) -> Vec<(String, String)> {
        if !self.sessions.contains_key(&session_id) {
            return vec![];
        }
        self.channels
            .iter()
            .filter(|ch| ch.members.contains(&session_id))
            .map(|ch| (ch.server_id.clone(), ch.name.clone()))
            .collect()
    }
    /// Get a session by ID.
    pub fn get_session(&self, session_id: ConnectionId) -> Option<Arc<UserSession>> {
        self.sessions.get(&session_id).map(|s| s.clone())
    }
    /// Get the database pool (if configured).
    pub fn get_db(&self) -> Option<SqlitePool> {
        self.db.clone()
    }
    pub(super) fn community_service(
        &self,
    ) -> Result<crate::engine::community_service::CommunityService, String> {
        Ok(crate::engine::community_service::CommunityService::new(
            self.db.clone().ok_or("No database configured")?,
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        ))
    }
    pub(super) fn moderation_service(
        &self,
    ) -> Result<crate::engine::moderation::ModerationService, String> {
        Ok(crate::engine::moderation::ModerationService::new(
            self.db.clone().ok_or_else(moderation_dependency)?,
            self.auth.get().ok_or_else(moderation_dependency)?.clone(),
            self.write_admission
                .as_ref()
                .ok_or_else(moderation_dependency)?
                .clone(),
        ))
    }
    pub(super) fn organization_service(
        &self,
    ) -> Result<crate::engine::organization::OrganizationService, String> {
        Ok(crate::engine::organization::OrganizationService::new(
            self.db.as_ref().ok_or("No database configured")?.clone(),
            self.auth.get().ok_or("Authentication unavailable")?.clone(),
            self.write_admission
                .as_ref()
                .ok_or("Write admission unavailable")?
                .clone(),
        ))
    }
    pub(super) fn media_service(
        &self,
    ) -> Result<crate::engine::media_service::MediaService, String> {
        Ok(crate::engine::media_service::MediaService::new(
            self.db
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: media dependency unavailable")?
                .clone(),
            self.auth
                .get()
                .ok_or("DEPENDENCY_UNAVAILABLE: media dependency unavailable")?
                .clone(),
            self.write_admission
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: media dependency unavailable")?
                .clone(),
        ))
    }
    pub(super) fn account_service(&self) -> Result<crate::engine::account::AccountService, String> {
        Ok(crate::engine::account::AccountService::new(
            self.db
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: account dependency unavailable")?
                .clone(),
            self.auth
                .get()
                .ok_or("DEPENDENCY_UNAVAILABLE: account dependency unavailable")?
                .clone(),
            self.write_admission
                .as_ref()
                .ok_or("DEPENDENCY_UNAVAILABLE: account dependency unavailable")?
                .clone(),
        ))
    }
}
