use super::ChatEngine;

impl ChatEngine {
    pub async fn list_server_folders_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<crate::engine::account::ServerFolder>, String> {
        self.account_service()?.list_server_folders(actor).await
    }
    pub async fn current_account_profile(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Option<crate::engine::account::AccountProfile>, String> {
        self.account_service()?.current_profile(actor).await
    }
    pub async fn public_account_profile(
        &self,
        nickname: &str,
    ) -> Result<Option<crate::engine::account::PublicAccountProfile>, String> {
        self.account_service()?.public_profile(nickname).await
    }
    pub async fn list_irc_tokens_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
    ) -> Result<Vec<crate::engine::account::IrcToken>, String> {
        self.account_service()?.list_irc_tokens(actor).await
    }
    pub async fn public_invite_preview(
        &self,
        code: &str,
    ) -> Result<
        Option<crate::engine::community_service::PublicInvitePreview>,
        crate::engine::community_service::PublicInvitePreviewError,
    > {
        self.community_service()
            .map_err(|_| {
                crate::engine::community_service::PublicInvitePreviewError::DependencyUnavailable
            })?
            .public_invite_preview(code)
            .await
    }
    pub async fn discover_public_servers(
        &self,
        category: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<crate::db::models::ServerRow>, String> {
        self.community_service()?
            .discover_public(category, limit, offset)
            .await
            .map_err(String::from)
    }
    pub async fn replace_server_folders_for_actor(
        &self,
        actor: &crate::auth::authority::Actor,
        folders: &[crate::engine::account::ServerFolder],
    ) -> Result<(), String> {
        self.account_service()?
            .replace_server_folders(actor, folders)
            .await
    }
    /// Resolve a channel name within a server to its channel ID.
    pub fn resolve_channel_id(
        &self,
        server_id: &str,
        channel_name: &str,
    ) -> Result<String, String> {
        self.channel_name_index
            .get(&(server_id.to_string(), channel_name.to_string()))
            .map(|r| r.clone())
            .ok_or(format!("No such channel: {channel_name}"))
    }
}
