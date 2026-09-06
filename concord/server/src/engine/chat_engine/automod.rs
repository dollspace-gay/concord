use super::{
    AutomodRuleInfo, ChatEngine, ChatEvent, ConnectionId, CreateAutomodRuleRequest,
    UpdateAutomodRuleRequest, moderation_unauthenticated, referenced_server_id,
};

impl ChatEngine {
    /// Create an automod rule.
    pub async fn create_automod_rule(
        &self,
        session_id: ConnectionId,
        params: &CreateAutomodRuleRequest<'_>,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let server_id = params.server_id;
        let name = params.name;
        let rule_type = params.rule_type;
        let config = params.config;
        let action_type = params.action_type;
        let timeout_duration_seconds = params.timeout_duration_seconds;
        let rule_id = self
            .moderation_service()?
            .create_automod_rule(
                &actor,
                &crate::engine::moderation::CreateAutomodRule {
                    server_id: &referenced_server_id(server_id)?,
                    name,
                    rule_type,
                    config,
                    action_type,
                    timeout_duration_seconds,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;

        let rule = AutomodRuleInfo {
            id: rule_id,
            name: name.to_string(),
            enabled: true,
            rule_type: rule_type.to_string(),
            config: config.to_string(),
            action_type: action_type.to_string(),
            timeout_duration_seconds,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleUpdate {
                server_id: server_id.to_string(),
                rule,
            });
        }

        Ok(())
    }
    /// Update an automod rule.
    pub async fn update_automod_rule(
        &self,
        session_id: ConnectionId,
        params: &UpdateAutomodRuleRequest<'_>,
    ) -> Result<(), String> {
        let server_id = params.server_id;
        let rule_id = params.rule_id;
        let name = params.name;
        let enabled = params.enabled;
        let config = params.config;
        let action_type = params.action_type;
        let timeout_duration_seconds = params.timeout_duration_seconds;

        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rule_type = self
            .moderation_service()?
            .update_automod_rule(
                &actor,
                &crate::engine::moderation::UpdateAutomodRule {
                    server_id: &referenced_server_id(server_id)?,
                    rule_id,
                    name,
                    enabled,
                    config,
                    action_type,
                    timeout_duration_seconds,
                },
            )
            .await
            .map_err(|error| error.wire_message())?;

        let rule = AutomodRuleInfo {
            id: rule_id.to_string(),
            name: name.to_string(),
            enabled,
            rule_type,
            config: config.to_string(),
            action_type: action_type.to_string(),
            timeout_duration_seconds,
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleUpdate {
                server_id: server_id.to_string(),
                rule,
            });
        }

        Ok(())
    }
    /// Delete an automod rule.
    pub async fn delete_automod_rule(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        rule_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        self.moderation_service()?
            .delete_automod_rule(&actor, &referenced_server_id(server_id)?, rule_id)
            .await
            .map_err(|error| error.wire_message())?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleDelete {
                server_id: server_id.to_string(),
                rule_id: rule_id.to_string(),
            });
        }

        Ok(())
    }
    /// List automod rules for a server.
    pub async fn list_automod_rules(
        &self,
        session_id: ConnectionId,
        server_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .get_authenticated_actor(session_id)
            .ok_or_else(moderation_unauthenticated)?;
        let rows = self
            .moderation_service()?
            .list_automod_rules(&actor, &referenced_server_id(server_id)?)
            .await
            .map_err(|error| error.wire_message())?;

        let rules: Vec<AutomodRuleInfo> = rows
            .into_iter()
            .map(|r| AutomodRuleInfo {
                id: r.id,
                name: r.name,
                enabled: r.enabled != 0,
                rule_type: r.rule_type,
                config: r.config,
                action_type: r.action_type,
                timeout_duration_seconds: r.timeout_duration_seconds,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::AutomodRuleList {
                server_id: server_id.to_string(),
                rules,
            });
        }

        Ok(())
    }
}
