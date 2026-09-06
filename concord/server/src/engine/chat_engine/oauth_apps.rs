use super::{ChatEngine, ChatEvent, ConnectionId, OAuth2AppInfo, Utc, Uuid};

impl ChatEngine {
    /// Create an OAuth2 application.
    pub async fn create_oauth2_app(
        &self,
        session_id: ConnectionId,
        name: &str,
        description: Option<&str>,
        redirect_uris: &[String],
        client_type: &str,
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        if !matches!(client_type, "confidential" | "public")
            || name.trim().is_empty()
            || name.len() > 100
            || description.is_some_and(|value| value.len() > 1_000)
            || redirect_uris.is_empty()
            || redirect_uris.len() > 10
        {
            return Err("Invalid OAuth2 application registration".into());
        }
        let mut exact_redirects = Vec::with_capacity(redirect_uris.len());
        for redirect_uri in redirect_uris {
            if redirect_uri.len() > 2_048 || redirect_uri.contains('#') {
                return Err("Invalid OAuth2 redirect URI".into());
            }
            let parsed = reqwest::Url::parse(redirect_uri)
                .map_err(|_| "Invalid OAuth2 redirect URI".to_string())?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err("OAuth2 redirect URIs must use HTTPS without credentials".into());
            }
            if !exact_redirects.contains(redirect_uri) {
                exact_redirects.push(redirect_uri.clone());
            }
        }
        let id = Uuid::new_v4().to_string();
        let raw_secret =
            (client_type == "confidential").then(|| format!("secret_{}", Uuid::new_v4()));
        let auth = self
            .auth
            .get()
            .ok_or("Credential authority is not configured")?;
        let secret_hash = match &raw_secret {
            Some(secret) => auth
                .hash_secret(secret.clone())
                .await
                .map_err(|_| "OAuth2 credential service unavailable".to_string())?,
            None => String::new(),
        };
        let uris_json = serde_json::to_string(&exact_redirects)
            .map_err(|e| format!("Invalid redirect URIs: {e}"))?;

        use crate::db::models::CreateOAuth2AppParams;
        let params = CreateOAuth2AppParams {
            id: &id,
            name,
            description: description.unwrap_or(""),
            icon_url: None,
            owner_id: &user_id,
            client_secret: &secret_hash,
            redirect_uris: &uris_json,
            scopes: "identify servers.read",
            client_type,
        };

        crate::db::queries::oauth2::create_app(pool, &params)
            .await
            .map_err(|e| format!("Failed to create OAuth2 app: {e}"))?;

        if let Some(session) = self.get_session(session_id) {
            let app = OAuth2AppInfo {
                id: id.clone(),
                name: name.to_string(),
                description: description.unwrap_or("").to_string(),
                icon_url: None,
                owner_id: user_id,
                redirect_uris: exact_redirects,
                scopes: "identify servers.read".to_string(),
                is_public: client_type == "public",
                created_at: Utc::now().to_rfc3339(),
            };
            let _ = session.send(ChatEvent::OAuth2AppUpdate { app });
            let _ = session.send(ChatEvent::ServerNotice {
                message: raw_secret.map_or_else(
                    || format!("Public OAuth2 app created. Client ID: {id}"),
                    |secret| {
                        format!("OAuth2 app created! Client ID: {id}, Client Secret: {secret}")
                    },
                ),
            });
        }

        Ok(())
    }
    /// List OAuth2 apps owned by the current user.
    pub async fn list_oauth2_apps(&self, session_id: ConnectionId) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let rows = crate::db::queries::oauth2::list_apps_by_owner(pool, &user_id)
            .await
            .map_err(|e| format!("Failed to list OAuth2 apps: {e}"))?;

        let apps: Vec<OAuth2AppInfo> = rows
            .into_iter()
            .map(|r| {
                let uris: Vec<String> = serde_json::from_str(&r.redirect_uris).unwrap_or_default();
                OAuth2AppInfo {
                    id: r.id,
                    name: r.name,
                    description: r.description,
                    icon_url: r.icon_url,
                    owner_id: r.owner_id,
                    redirect_uris: uris,
                    scopes: r.scopes,
                    is_public: r.is_public != 0,
                    created_at: r.created_at,
                }
            })
            .collect();

        if let Some(session) = self.get_session(session_id) {
            let _ = session.send(ChatEvent::OAuth2AppList { apps });
        }

        Ok(())
    }
    /// Delete an OAuth2 app. Only the owner can delete.
    pub async fn delete_oauth2_app(
        &self,
        session_id: ConnectionId,
        app_id: &str,
    ) -> Result<(), String> {
        let user_id = self.get_user_id(session_id)?;

        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let app = crate::db::queries::oauth2::get_app(pool, app_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("OAuth2 app not found")?;

        if app.owner_id != user_id {
            return Err("You can only delete your own apps".into());
        }

        crate::db::queries::oauth2::delete_app(pool, app_id)
            .await
            .map_err(|e| format!("Failed to delete app: {e}"))?;

        self.list_oauth2_apps(session_id).await
    }
}
