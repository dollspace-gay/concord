use super::{ChatEngine, ChatEvent, ConnectionId, ServerCommunityInfo, referenced_server_id};

impl ChatEngine {
    /// Update community/discovery settings. Requires MANAGE_SERVER permission.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_community_settings(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        description: Option<&str>,
        is_discoverable: bool,
        welcome_message: Option<&str>,
        rules_text: Option<&str>,
        category: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or("UNAUTHENTICATED: authentication required")?;
        let rules_accepted = self
            .community_service()?
            .update_community(
                &actor,
                &crate::engine::community_service::UpdateCommunityParams {
                    server_id: &referenced_server_id(server_id)?,
                    description,
                    discoverable: is_discoverable,
                    welcome: welcome_message,
                    rules: rules_text,
                    category,
                },
            )
            .await
            .map_err(String::from)?;

        let community = ServerCommunityInfo {
            server_id: server_id.to_string(),
            description: description.map(String::from),
            is_discoverable,
            welcome_message: welcome_message.map(String::from),
            rules_text: rules_text.map(String::from),
            category: category.map(String::from),
            rules_accepted: Some(rules_accepted),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::ServerCommunity { community });
        }

        Ok(())
    }
    /// Get community/discovery settings for a server. Requires VIEW_CHANNELS permission.
    pub async fn get_community_settings(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (server, rules_accepted, stamp) = self
            .community_service()?
            .get_community(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let community = ServerCommunityInfo {
            server_id: server.id,
            description: server.description,
            is_discoverable: server.is_discoverable != 0,
            welcome_message: server.welcome_message,
            rules_text: server.rules_text,
            category: server.category,
            rules_accepted: Some(rules_accepted),
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::ServerCommunity { community },
                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                    stamp,
                ])),
            );
        }

        Ok(())
    }
    /// Discover public servers, optionally filtered by category. No permission needed.
    pub async fn discover_servers(
        &self,
        session_id: ConnectionId,
        category: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let rows = self
            .community_service()?
            .discover(&actor, category)
            .await
            .map_err(String::from)?;

        let servers: Vec<ServerCommunityInfo> = rows
            .into_iter()
            .map(|r| ServerCommunityInfo {
                server_id: r.id,
                description: r.description,
                is_discoverable: r.is_discoverable != 0,
                welcome_message: r.welcome_message,
                rules_text: r.rules_text,
                category: r.category,
                rules_accepted: None,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::DiscoverServers { servers });
        }

        Ok(())
    }
    /// Accept server rules as a member.
    pub async fn accept_rules(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.community_service()?
            .accept_rules(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        Ok(())
    }
    pub async fn set_vanity_code(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        vanity_code: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.community_service()?
            .set_vanity_code(&actor, &referenced_server_id(server_id)?, vanity_code)
            .await
            .map_err(String::from)
    }
}
