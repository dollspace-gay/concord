#[cfg(test)]
use super::normalize_channel_name;
use super::{ChatEngine, ConnectionId};

impl ChatEngine {
    /// Mark a channel as read for a user, up to a specific message ID.
    #[cfg(test)]
    pub async fn mark_read(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let stamp = crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_stamped(
                auth,
                &actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        crate::db::queries::messages::mark_channel_read(pool, user_id, &channel_id, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        if !self.authorization_stamp_is_current(&actor, &stamp).await {
            return Err("resource unavailable".into());
        }

        Ok(())
    }
    /// Get unread counts for all channels in a server for a user.
    pub async fn get_unread_counts(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<
        (
            Vec<crate::engine::events::UnreadCount>,
            Vec<crate::engine::authorization::AuthorizationStamp>,
        ),
        String,
    > {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let user_id = session.user_id.as_deref().ok_or("AUTH_REQUIRED")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows = crate::db::queries::messages::get_unread_counts(pool, user_id, server_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        // Map channel_id -> channel_name
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        let mut counts = Vec::new();
        let mut stamps = Vec::new();
        for r in rows {
            let Ok(stamp) = authorization
                .authorize_actor_stamped(
                    auth,
                    &actor,
                    &r.channel_id,
                    crate::engine::authorization::ChannelAction::ReadHistory,
                )
                .await
            else {
                continue;
            };
            stamps.push(stamp);
            if let Some(name) = self.channels.get(&r.channel_id).map(|ch| ch.name.clone()) {
                counts.push(crate::engine::events::UnreadCount {
                    channel_name: name,
                    count: r.unread_count,
                });
            }
        }
        for stamp in &stamps {
            if !self.authorization_stamp_is_current(&actor, stamp).await {
                return Err("resource unavailable".into());
            }
        }
        Ok((counts, stamps))
    }
}
