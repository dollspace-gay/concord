use super::{ChatEngine, ChatEvent, Row};

impl ChatEngine {
    /// Get a user's full profile.
    pub async fn get_user_profile(
        &self,
        actor: &crate::auth::authority::Actor,
        user_id: &str,
    ) -> Result<
        (
            crate::engine::events::UserProfileInfo,
            Option<crate::engine::authorization::AuthorizationStamp>,
        ),
        String,
    > {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let mut transaction = pool.begin().await.map_err(|_| "resource unavailable")?;
        auth.validate_actor_in(&mut transaction, actor)
            .await
            .map_err(|_| "resource unavailable")?;
        let shared_server: Option<(String, i64)> = if actor.user_id().as_str() == user_id {
            None
        } else {
            sqlx::query_as(
                "SELECT s.id,s.authorization_version FROM servers s \
                 JOIN server_members requester ON requester.server_id=s.id AND requester.user_id=? \
                 JOIN server_members target ON target.server_id=s.id AND target.user_id=? \
                 WHERE NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id IN (?,?)) \
                 ORDER BY s.id LIMIT 1",
            )
            .bind(actor.user_id().as_str())
            .bind(user_id)
            .bind(actor.user_id().as_str())
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| "resource unavailable")?
        };
        if actor.user_id().as_str() != user_id && shared_server.is_none() {
            return Err("resource unavailable".into());
        }
        let row = sqlx::query(
            "SELECT u.id,u.username,u.avatar_url,p.bio,p.pronouns,p.banner_url,u.created_at \
             FROM users u LEFT JOIN user_profiles p ON p.user_id=u.id \
             WHERE u.id=? AND u.disabled_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "resource unavailable")?
        .ok_or_else(|| "resource unavailable".to_string())?;
        let profile = crate::engine::events::UserProfileInfo {
            user_id: row.get(0),
            username: row.get(1),
            avatar_url: row.get(2),
            bio: row.get(3),
            pronouns: row.get(4),
            banner_url: row.get(5),
            created_at: row.get(6),
        };
        let stamp = shared_server.map(|(server_id, server_version)| {
            crate::engine::authorization::AuthorizationStamp {
                server_id,
                server_version,
                channel_versions: Vec::new(),
            }
        });
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable")?;
        Ok((profile, stamp))
    }
    pub fn broadcast_profile_update(&self, profile: crate::engine::events::UserProfileInfo) {
        for server in self.servers.iter() {
            if !server.member_user_ids.contains(&profile.user_id) {
                continue;
            }
            let mut notified = std::collections::HashSet::new();
            for channel_id in &server.channel_ids {
                if let Some(channel) = self.channels.get(channel_id) {
                    for session_id in &channel.members {
                        if notified.insert(*session_id)
                            && let Some(session) = self.sessions.get(session_id)
                        {
                            let _ = session.send_guarded(
                                ChatEvent::UserProfile {
                                    profile: profile.clone(),
                                },
                                Some(
                                    crate::engine::user_session::DeliveryGuard::ServerMembership(
                                        vec![server.id.clone()],
                                    ),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    pub(super) fn profile_sync_service(
        &self,
    ) -> Result<
        crate::engine::profile_sync::ProfileSyncService,
        crate::engine::profile_sync::ProfileSyncError,
    > {
        Ok(crate::engine::profile_sync::ProfileSyncService::new(
            self.db
                .as_ref()
                .ok_or(crate::engine::profile_sync::ProfileSyncError::DependencyUnavailable)?
                .clone(),
            self.auth
                .get()
                .ok_or(crate::engine::profile_sync::ProfileSyncError::DependencyUnavailable)?
                .clone(),
            self.write_admission
                .as_ref()
                .ok_or(crate::engine::profile_sync::ProfileSyncError::DependencyUnavailable)?
                .clone(),
        ))
    }
    pub async fn verified_atproto_profile_did(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<String, crate::engine::profile_sync::ProfileSyncError> {
        self.profile_sync_service()?.verified_did(actor).await
    }
    pub async fn apply_atproto_profile_sync(
        &self,
        actor: &crate::auth::authority::Actor,
        expected_did: &str,
        profile: &crate::engine::profile_sync::BlueskyProfileSyncInput<'_>,
    ) -> Result<crate::engine::events::UserProfileInfo, crate::engine::profile_sync::ProfileSyncError>
    {
        let updated = self
            .profile_sync_service()?
            .apply(actor, expected_did, profile)
            .await?;
        self.broadcast_profile_update(updated.clone());
        Ok(updated)
    }
    pub async fn atproto_identity_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        user_id: &str,
    ) -> Result<
        (
            Option<crate::engine::profile_sync::AtprotoIdentity>,
            Option<crate::engine::authorization::AuthorizationStamp>,
        ),
        crate::engine::profile_sync::ProfileSyncError,
    > {
        self.profile_sync_service()?
            .identity_for_actor(actor, user_id)
            .await
    }
    pub async fn atproto_sync_enabled_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<bool, crate::engine::profile_sync::ProfileSyncError> {
        self.profile_sync_service()?.sync_enabled(actor).await
    }
    pub async fn set_atproto_sync_enabled_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        enabled: bool,
    ) -> Result<(), crate::engine::profile_sync::ProfileSyncError> {
        self.profile_sync_service()?
            .set_sync_enabled(actor, enabled)
            .await
    }
}
