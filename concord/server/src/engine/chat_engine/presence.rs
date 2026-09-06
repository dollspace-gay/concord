use super::{
    ChatEngine, ChatEvent, ConnectionId, PresenceProjectionRow, referenced_server_id,
    server_member_display_identity, warn,
};

impl ChatEngine {
    /// Update a user's presence and broadcast to members of shared servers.
    pub async fn set_presence(
        &self,
        session_id: ConnectionId,
        status: &str,
        custom_status: Option<&str>,
        status_emoji: Option<&str>,
    ) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "Not authenticated".to_string())?;
        let user_id = actor.user_id().as_str().to_owned();

        // Validate status
        match status {
            "online" | "idle" | "dnd" | "invisible" => {}
            _ => return Err("Invalid status. Must be: online, idle, dnd, invisible".into()),
        }
        if custom_status.is_some_and(|value| value.chars().count() > 128) {
            return Err("Custom status must be 128 characters or less".into());
        }
        if status_emoji.is_some_and(|value| value.chars().count() > 64) {
            return Err("Status emoji must be 64 characters or less".into());
        }

        let pool = self.db.as_ref().ok_or("Database unavailable")?;
        let writes = self
            .write_admission
            .as_ref()
            .ok_or("Write admission unavailable")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let (_permit, mut transaction) = writes.begin().await.map_err(|e| e.to_string())?;
        auth.validate_actor_in(&mut transaction, &actor)
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        sqlx::query(
            "INSERT INTO user_presence (user_id,status,requested_status,custom_status,status_emoji,last_seen_at,updated_at) \
             VALUES (?,?,?,?,?,datetime('now'),datetime('now')) \
             ON CONFLICT(user_id) DO UPDATE SET status=excluded.status,requested_status=excluded.requested_status, \
             custom_status=excluded.custom_status,status_emoji=excluded.status_emoji,last_seen_at=datetime('now'),updated_at=datetime('now')",
        )
        .bind(&user_id)
        .bind(status)
        .bind(status)
        .bind(custom_status)
        .bind(status_emoji)
        .execute(&mut *transaction)
        .await
        .map_err(|e| format!("Failed to update presence: {e}"))?;
        transaction
            .commit()
            .await
            .map_err(|e| format!("Failed to update presence: {e}"))?;

        let _ = session.send(ChatEvent::OwnPresence {
            requested_status: status.to_owned(),
            effective_status: if status == "invisible" {
                "offline"
            } else {
                status
            }
            .to_owned(),
            custom_status: custom_status.map(str::to_owned),
            status_emoji: status_emoji.map(str::to_owned),
        });

        // Broadcast to all servers the user is a member of
        let server_ids: Vec<String> = self
            .servers
            .iter()
            .filter(|server| server.member_user_ids.contains(&user_id))
            .map(|server| server.id.clone())
            .collect();
        for server_id in server_ids {
            let identity = server_member_display_identity(pool, &server_id, &user_id)
                .await
                .map_err(|error| {
                    warn!(%error, %server_id, %user_id, "presence identity query failed");
                    "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
                })?;
            if let (Some((nickname, avatar_url)), Some(server)) =
                (identity, self.servers.get(&server_id))
            {
                let presence = crate::engine::events::PresenceInfo {
                    user_id: user_id.clone(),
                    nickname,
                    avatar_url,
                    status: if status == "invisible" {
                        "offline".into()
                    } else {
                        status.into()
                    },
                    custom_status: (status != "invisible")
                        .then(|| custom_status.map(str::to_owned))
                        .flatten(),
                    status_emoji: (status != "invisible")
                        .then(|| status_emoji.map(str::to_owned))
                        .flatten(),
                };
                let event = ChatEvent::PresenceUpdate {
                    server_id: server_id.clone(),
                    presence,
                };
                // Send to all sessions in this server's channels, deduplicated per server
                let mut notified = std::collections::HashSet::new();
                for channel_id in server.channel_ids.iter() {
                    if let Some(channel) = self.channels.get(channel_id) {
                        for &member_sid in &channel.members {
                            if member_sid != session_id
                                && notified.insert(member_sid)
                                && let Some(s) = self.sessions.get(&member_sid)
                            {
                                let _ = s.send_guarded(
                                    event.clone(),
                                    Some(crate::engine::user_session::DeliveryGuard::ServerMembership(
                                        vec![server_id.clone()],
                                    )),
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
    pub async fn send_own_presence(&self, session_id: ConnectionId) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let pool = self.db.as_ref().ok_or("Database unavailable")?;
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT requested_status,custom_status,status_emoji FROM user_presence WHERE user_id=?",
        )
        .bind(actor.user_id().as_str())
        .fetch_optional(pool)
        .await
        .map_err(|_| "Database unavailable".to_string())?;
        let (requested_status, custom_status, status_emoji) =
            row.unwrap_or_else(|| ("online".to_owned(), None, None));
        let effective_status = if requested_status == "invisible" {
            "offline".to_owned()
        } else {
            requested_status.clone()
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::OwnPresence {
                requested_status,
                effective_status,
                custom_status,
                status_emoji,
            });
        }
        Ok(())
    }
    /// Get presence list for all members of a server.
    pub async fn get_server_presences(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<Vec<crate::engine::events::PresenceInfo>, String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|_| "Not authenticated".to_string())?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let mut tx = pool.begin().await.map_err(|error| {
            warn!(%error, "presence projection database begin failed");
            "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
        })?;
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .server_actor_permissions_in(&mut tx, auth, &actor, server_id)
            .await
            .map_err(|error| match error {
                crate::engine::authorization::AuthorizationError::Database(error) => {
                    warn!(%error, "presence projection authorization database failed");
                    "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
                }
                crate::engine::authorization::AuthorizationError::Authentication(_) => {
                    "UNAUTHENTICATED: authentication required".to_string()
                }
                crate::engine::authorization::AuthorizationError::Unavailable => {
                    "resource unavailable".to_string()
                }
            })?;
        let rows: Vec<PresenceProjectionRow> = sqlx::query_as(
            "SELECT sm.user_id AS user_id, \
                    COALESCE(NULLIF(sm.nickname,''),u.username) AS nickname, \
                    COALESCE(sm.avatar_url,u.avatar_url) AS avatar_url, \
                    p.requested_status AS requested_status, \
                    p.custom_status AS custom_status,p.status_emoji AS status_emoji \
             FROM server_members sm JOIN users u ON u.id=sm.user_id \
             LEFT JOIN user_presence p ON p.user_id=sm.user_id \
             WHERE sm.server_id=? AND NOT EXISTS( \
                 SELECT 1 FROM bans b WHERE b.server_id=sm.server_id AND b.user_id=sm.user_id \
             ) ORDER BY sm.user_id",
        )
        .bind(server_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| {
            warn!(%error, "presence projection database query failed");
            "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
        })?;
        tx.commit().await.map_err(|error| {
            warn!(%error, "presence projection database commit failed");
            "DEPENDENCY_UNAVAILABLE: presence dependency unavailable".to_string()
        })?;
        Ok(rows
            .into_iter()
            .map(
                |PresenceProjectionRow {
                     user_id,
                     nickname,
                     avatar_url,
                     requested_status,
                     custom_status,
                     status_emoji,
                 }| {
                    let live = self
                        .user_connections
                        .get(&user_id)
                        .is_some_and(|connections| !connections.is_empty());
                    let requested_status = requested_status.unwrap_or_else(|| "online".into());
                    let visible = live && requested_status != "invisible";
                    crate::engine::events::PresenceInfo {
                        user_id,
                        nickname,
                        avatar_url,
                        status: if visible {
                            requested_status
                        } else {
                            "offline".into()
                        },
                        custom_status: visible.then_some(custom_status).flatten(),
                        status_emoji: visible.then_some(status_emoji).flatten(),
                    }
                },
            )
            .collect())
    }
    /// Set a user's server-specific display name.
    pub async fn set_server_nickname(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        nickname: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let base_display_name = self
            .get_session(session_id)
            .map(|session| session.nickname.clone())
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let user_id = actor.user_id().as_str().to_owned();
        let server_avatar_url = self
            .organization_service()?
            .set_server_nickname(&actor, &referenced_server_id(server_id)?, nickname)
            .await
            .map_err(String::from)?;

        // Broadcast nickname change
        let event = ChatEvent::ServerNicknameUpdate {
            server_id: server_id.to_string(),
            user_id: user_id.clone(),
            nickname: nickname.map(str::trim).map(str::to_owned),
            display_name: nickname
                .map(str::trim)
                .unwrap_or(&base_display_name)
                .to_owned(),
            server_avatar_url,
        };
        if let Some(server) = self.servers.get(server_id) {
            for channel_id in server.channel_ids.iter() {
                self.broadcast_to_channel(channel_id, &event, None);
            }
        }

        Ok(())
    }
    pub fn broadcast_server_member_identity(
        &self,
        server_id: &str,
        user_id: &str,
        nickname: Option<String>,
        display_name: String,
        avatar_url: Option<String>,
    ) {
        let event = ChatEvent::ServerNicknameUpdate {
            server_id: server_id.to_owned(),
            user_id: user_id.to_owned(),
            nickname,
            display_name,
            server_avatar_url: avatar_url,
        };
        if let Some(server) = self.servers.get(server_id) {
            for channel_id in &server.channel_ids {
                self.broadcast_to_channel(channel_id, &event, None);
            }
        }
    }
}
