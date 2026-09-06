use super::{ChatEngine, ChatEvent, ConnectionId, Permissions, TemplateInfo, referenced_server_id};

impl ChatEngine {
    /// Create a server template (snapshot of channels, categories, roles).
    /// Requires MANAGE_SERVER permission.
    pub async fn create_template(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let created = self
            .community_service()?
            .create_template(&actor, &referenced_server_id(server_id)?, name, description)
            .await
            .map_err(String::from)?;
        let template = TemplateInfo {
            id: created.id,
            name: name.to_string(),
            description: description.map(String::from),
            server_id: server_id.to_string(),
            created_by: actor.user_id().as_str().to_owned(),
            use_count: 0,
            created_at: created.created_at,
        };
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::TemplateUpdate {
                    server_id: server_id.to_string(),
                    template,
                },
                Some(
                    crate::engine::user_session::DeliveryGuard::ServerPermissions(vec![(
                        server_id.to_string(),
                        Permissions::MANAGE_SERVER,
                    )]),
                ),
            );
        }
        Ok(())
    }
    /// List templates for a server. Sends TemplateList to the session.
    pub async fn list_templates(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let (rows, stamp) = self
            .community_service()?
            .list_templates(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(String::from)?;

        let templates: Vec<TemplateInfo> = rows
            .into_iter()
            .map(|r| TemplateInfo {
                id: r.id,
                name: r.name,
                description: r.description,
                server_id: r.server_id,
                created_by: r.created_by,
                use_count: r.use_count,
                created_at: r.created_at,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::TemplateList {
                    server_id: server_id.to_string(),
                    templates,
                },
                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                    stamp,
                ])),
            );
        }

        Ok(())
    }
    /// Delete a template. Requires MANAGE_SERVER permission.
    pub async fn delete_template(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        template_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        self.community_service()?
            .delete_template(&actor, &referenced_server_id(server_id)?, template_id)
            .await
            .map_err(String::from)?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send_guarded(
                ChatEvent::TemplateDelete {
                    server_id: server_id.to_string(),
                    template_id: template_id.to_string(),
                },
                Some(
                    crate::engine::user_session::DeliveryGuard::ServerPermissions(vec![(
                        server_id.to_string(),
                        Permissions::MANAGE_SERVER,
                    )]),
                ),
            );
        }

        Ok(())
    }
    /// Atomically create a server from a safe, versioned template snapshot.
    pub async fn instantiate_template(
        &self,
        session_id: ConnectionId,
        template_id: &str,
        server_name: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))?;
        let server_id = self
            .community_service()?
            .instantiate_template(&actor, template_id, server_name)
            .await
            .map_err(String::from)?
            .into_inner();
        self.load_servers_from_db().await?;
        self.load_channels_from_db().await?;
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::TemplateInstantiated {
                template_id: template_id.to_string(),
                server_id,
            });
        }
        Ok(())
    }
}
