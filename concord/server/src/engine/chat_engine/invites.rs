use super::{
    ChatEngine, ChatEvent, ConnectionId, InviteInfo, referenced_channel_id, referenced_server_id,
};

impl ChatEngine {
    /// Create a server invite. Requires CREATE_INVITES permission.
    pub async fn create_invite(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        max_uses: Option<i32>,
        expires_at: Option<&str>,
        channel_id: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let server_resource_id = referenced_server_id(server_id)?;
        let channel_resource_id = channel_id.map(referenced_channel_id).transpose()?;
        let created = self
            .community_service()?
            .create_invite(
                &actor,
                &server_resource_id,
                max_uses,
                expires_at,
                channel_resource_id.as_ref(),
            )
            .await
            .map_err(String::from)?;

        let invite = InviteInfo {
            id: created.id,
            code: created.code,
            server_id: server_id.to_string(),
            created_by: actor.user_id().as_str().to_owned(),
            max_uses,
            use_count: 0,
            expires_at: expires_at.map(String::from),
            channel_id: channel_id.map(String::from),
            created_at: created.created_at,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteCreate {
                server_id: server_id.to_string(),
                invite,
            });
        }

        Ok(())
    }
    /// List invites for a server. Requires MANAGE_SERVER permission.
    pub async fn list_invites(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (rows, stamp) = self
            .community_service()?
            .list_invites(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let invites: Vec<InviteInfo> = rows
            .into_iter()
            .map(|r| InviteInfo {
                id: r.id,
                code: r.code,
                server_id: r.server_id,
                created_by: r.created_by,
                max_uses: r.max_uses,
                use_count: r.use_count,
                expires_at: r.expires_at,
                channel_id: r.channel_id,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::InviteList {
                    server_id: server_id.to_string(),
                    invites,
                },
                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                    stamp,
                ])),
            );
        }

        Ok(())
    }
    /// Delete an invite. Requires MANAGE_SERVER permission.
    pub async fn delete_invite(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        invite_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        self.community_service()?
            .delete_invite(&actor, &referenced_server_id(server_id)?, invite_id)
            .await
            .map_err(String::from)?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::InviteDelete {
                server_id: server_id.to_string(),
                invite_id: invite_id.to_string(),
            });
        }

        Ok(())
    }
    /// Use an invite code to join a server. Any authenticated user can use this.
    pub async fn use_invite(&self, session_id: ConnectionId, code: &str) -> Result<(), String> {
        let session = self.get_session(session_id).ok_or("Session not found")?;
        let user_id = session
            .user_id
            .as_deref()
            .ok_or("AUTH_REQUIRED")?
            .to_string();
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(|| "resource unavailable".to_string())?;
        let redeemed = self
            .community_service()?
            .redeem_invite(&actor, code)
            .await?;
        let server_id = redeemed.server_id.into_inner();
        if let Some(mut server) = self.servers.get_mut(&server_id) {
            server.member_user_ids.insert(user_id.clone());
        }

        // Auto-join default channel (#general)
        let default_channel = self
            .channel_name_index
            .get(&(server_id.clone(), "#general".to_string()))
            .map(|r| r.clone());
        if default_channel.is_some() {
            let _ = self.join_channel(session_id, &server_id, "#general").await;
        }

        // Send updated server list to the user
        let servers = self.list_servers_for_user(&user_id).await;
        let _ = session.send(ChatEvent::ServerList { servers });

        Ok(())
    }
}
