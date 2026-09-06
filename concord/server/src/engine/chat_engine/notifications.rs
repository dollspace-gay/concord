use super::{ChatEngine, ConnectionId, Permissions, UpdateNotificationSettingsParams, Uuid};

impl ChatEngine {
    /// Update notification settings for a user in a server or channel.
    pub async fn update_notification_settings(
        &self,
        session_id: ConnectionId,
        params: &UpdateNotificationSettingsParams<'_>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;

        match params.level {
            "all" | "mentions" | "none" | "default" => {}
            _ => return Err("Invalid level. Must be: all, mentions, none, default".into()),
        }

        let pool = self.db.as_ref().ok_or("No database configured")?;
        let id = Uuid::new_v4().to_string();
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|error| error.to_string())?;
        let authorization = crate::engine::authorization::AuthorizationService::new(pool.clone());
        if let Some(channel_id) = params.channel_id {
            authorization
                .authorize_actor_in(
                    &mut transaction,
                    self.auth.get().ok_or("Authentication unavailable")?,
                    &actor,
                    channel_id,
                    crate::engine::authorization::ChannelAction::View,
                )
                .await
                .map_err(|_| "resource unavailable".to_string())?;
            let actual_server: Option<String> =
                sqlx::query_scalar("SELECT server_id FROM channels WHERE id=?")
                    .bind(channel_id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(|_| "resource unavailable".to_string())?;
            if actual_server.as_deref() != Some(params.server_id) {
                return Err("resource unavailable".into());
            }
        } else {
            authorization
                .require_server_actor_in(
                    &mut transaction,
                    self.auth.get().ok_or("Authentication unavailable")?,
                    &actor,
                    params.server_id,
                    Permissions::VIEW_CHANNELS,
                )
                .await
                .map_err(|_| "resource unavailable".to_string())?;
        }
        let conflict = if params.channel_id.is_some() {
            " ON CONFLICT(user_id,channel_id) WHERE channel_id IS NOT NULL DO UPDATE SET "
        } else {
            " ON CONFLICT(user_id,server_id) WHERE server_id IS NOT NULL AND channel_id IS NULL DO UPDATE SET "
        };
        let sql = format!(
            "INSERT INTO notification_settings(id,user_id,server_id,channel_id,level, \
             suppress_everyone,suppress_roles,muted,mute_until,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,datetime('now')){conflict} \
             level=excluded.level,suppress_everyone=excluded.suppress_everyone, \
             suppress_roles=excluded.suppress_roles,muted=excluded.muted, \
             mute_until=excluded.mute_until,updated_at=datetime('now')"
        );
        // Only the literal conflict clause above is interpolated; all values are bound.
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&id)
            .bind(actor.user_id().as_str())
            .bind(params.server_id)
            .bind(params.channel_id)
            .bind(params.level)
            .bind(params.suppress_everyone as i32)
            .bind(params.suppress_roles as i32)
            .bind(params.muted as i32)
            .bind(params.mute_until)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        transaction
            .commit()
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        Ok(())
    }
    /// Get notification settings for a user in a server.
    pub async fn get_notification_settings(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<Vec<crate::engine::events::NotificationSettingInfo>, String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session.user_id.clone().ok_or("Not authenticated")?;

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let rows =
            crate::db::queries::notifications::get_notification_settings(pool, &user_id, server_id)
                .await
                .map_err(|e| format!("Failed to get notification settings: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|r| crate::engine::events::NotificationSettingInfo {
                id: r.id,
                server_id: r.server_id,
                channel_id: r.channel_id,
                level: r.level,
                suppress_everyone: r.suppress_everyone != 0,
                suppress_roles: r.suppress_roles != 0,
                muted: r.muted != 0,
                mute_until: r.mute_until,
            })
            .collect())
    }
}
