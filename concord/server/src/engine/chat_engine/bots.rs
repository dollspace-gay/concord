use super::{BotTokenInfo, ChatEngine, ChatEvent, ConnectionId, Permissions, Uuid};
use crate::engine::validation;

impl ChatEngine {
    /// Create a bot account. Only authenticated users can create bots.
    pub async fn create_bot(
        &self,
        session_id: ConnectionId,
        username: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), String> {
        let creator_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        validation::validate_nickname(username)?;

        let bot_user_id = Uuid::new_v4().to_string();
        crate::db::queries::bots::create_bot_user_owned(
            pool,
            &bot_user_id,
            username,
            avatar_url,
            &creator_id,
        )
        .await
        .map_err(|e| format!("Failed to create bot: {e}"))?;

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let bot_id = crate::auth::authority::UserId::from_stored(bot_user_id.clone())
            .map_err(|e| e.to_string())?;
        let issued = match auth
            .issue_bot_token(&bot_id, "Default", "bot messages")
            .await
        {
            Ok(issued) => issued,
            Err(error) => {
                let _ = crate::db::queries::bots::delete_bot_user(pool, &bot_user_id).await;
                return Err(format!("Failed to create bot token: {error}"));
            }
        };

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotCredentialCreated {
                bot_user_id: bot_user_id.clone(),
                token: issued.secret,
                credential: BotTokenInfo {
                    id: issued.token_id,
                    name: "Default".into(),
                    scopes: "bot messages".into(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    last_used: None,
                },
            });
        }
        self.list_owned_bots(session_id).await
    }
    pub async fn list_owned_bots(&self, session_id: ConnectionId) -> Result<(), String> {
        use sqlx::Row;
        let owner_id = self.get_user_id(session_id)?;
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let rows = sqlx::query(
            "SELECT u.id,u.username,u.avatar_url,i.server_id
             FROM bot_ownership o JOIN users u ON u.id=o.bot_user_id
             LEFT JOIN bot_installations i ON i.bot_user_id=u.id AND i.state='active'
             WHERE o.owner_user_id=? AND o.repair_required=0
             ORDER BY u.username,u.id,i.server_id",
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("Failed to list bots: {error}"))?;
        let mut bots: Vec<crate::engine::events::BotAccountInfo> = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            if bots.last().is_none_or(|bot| bot.id != id) {
                bots.push(crate::engine::events::BotAccountInfo {
                    id,
                    username: row.get(1),
                    avatar_url: row.get(2),
                    installed_server_ids: Vec::new(),
                });
            }
            if let Some(server_id) = row.get::<Option<String>, _>(3) {
                bots.last_mut()
                    .expect("bot was inserted")
                    .installed_server_ids
                    .push(server_id);
            }
        }
        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotAccountList { bots });
        }
        Ok(())
    }
    /// Create a new token for a bot. Caller must own the bot.
    pub async fn create_bot_token(
        &self,
        session_id: ConnectionId,
        bot_user_id: &str,
        name: &str,
        scopes: Option<&str>,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let caller_id = self.get_user_id(session_id)?;
        let owner_id = crate::db::queries::bots::bot_owner(pool, bot_user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if owner_id.as_deref() != Some(&caller_id) {
            return Err("FORBIDDEN: only the recorded bot owner may create credentials".into());
        }

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let bot_id =
            crate::auth::authority::UserId::from_stored(bot_user_id).map_err(|e| e.to_string())?;
        let issued = auth
            .issue_bot_token(&bot_id, name, scopes.unwrap_or("bot messages"))
            .await
            .map_err(|e| format!("Failed to create bot token: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotCredentialCreated {
                bot_user_id: bot_user_id.to_owned(),
                token: issued.secret,
                credential: BotTokenInfo {
                    id: issued.token_id,
                    name: name.to_owned(),
                    scopes: scopes.unwrap_or("bot messages").to_owned(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    last_used: None,
                },
            });
        }

        Ok(())
    }
    /// List bot tokens (without hashes).
    pub async fn list_bot_tokens(
        &self,
        session_id: ConnectionId,
        bot_user_id: &str,
    ) -> Result<(), String> {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let caller_id = self.get_user_id(session_id)?;
        let owner_id = crate::db::queries::bots::bot_owner(pool, bot_user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if owner_id.as_deref() != Some(&caller_id) {
            return Err("FORBIDDEN: only the recorded bot owner may list credentials".into());
        }
        let rows = crate::db::queries::bots::list_bot_tokens(pool, bot_user_id)
            .await
            .map_err(|e| format!("Failed to list bot tokens: {e}"))?;

        let tokens: Vec<BotTokenInfo> = rows
            .into_iter()
            .map(|r| BotTokenInfo {
                id: r.id,
                name: r.name,
                scopes: r.scopes,
                created_at: r.created_at,
                last_used: r.last_used,
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::BotTokenList {
                bot_user_id: bot_user_id.to_string(),
                tokens,
            });
        }

        Ok(())
    }
    /// Delete a bot token.
    pub async fn delete_bot_token(
        &self,
        session_id: ConnectionId,
        token_id: &str,
    ) -> Result<(), String> {
        let actor = self
            .actor_for_session(session_id)
            .map_err(|error| error.to_string())?;
        let user_id = actor.user_id().as_str().to_owned();

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let owner_id = crate::db::queries::bots::bot_token_owner(pool, token_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        if owner_id.as_deref() != Some(&user_id) {
            return Err("FORBIDDEN: only the recorded bot owner may revoke credentials".into());
        }
        let bot_user_id: String = sqlx::query_scalar("SELECT user_id FROM bot_tokens WHERE id=?")
            .bind(token_id)
            .fetch_one(pool)
            .await
            .map_err(|error| error.to_string())?;

        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        auth.revoke_bot_token(token_id)
            .await
            .map_err(|e| format!("Failed to revoke bot token: {e}"))?;

        self.list_bot_tokens(session_id, &bot_user_id).await?;
        Ok(())
    }
    /// Add a bot to a server. Requires MANAGE_SERVER permission.
    pub async fn add_bot_to_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        bot_user_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::add_bot_to_server_with_grants(
            pool,
            server_id,
            bot_user_id,
            &self.get_user_id(session_id)?,
            "commands messages",
        )
        .await
        .map_err(|e| format!("Failed to add bot to server: {e}"))?;

        self.list_owned_bots(session_id).await
    }
    /// Remove a bot from a server. Requires MANAGE_SERVER permission.
    pub async fn remove_bot_from_server(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        bot_user_id: &str,
    ) -> Result<(), String> {
        self.require_permission(session_id, server_id, None, Permissions::MANAGE_SERVER)
            .await?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        crate::db::queries::bots::remove_bot_from_server(pool, server_id, bot_user_id)
            .await
            .map_err(|e| format!("Failed to remove bot from server: {e}"))?;

        self.list_owned_bots(session_id).await
    }
}
