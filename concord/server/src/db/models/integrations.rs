use super::{Deserialize, Serialize};

/// A bot API token.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BotTokenRow {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub name: String,
    pub scopes: String,
    pub created_at: String,
    pub last_used: Option<String>,
}

/// A webhook (incoming or outgoing).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookRow {
    pub id: String,
    pub server_id: String,
    pub channel_id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub webhook_type: String,
    pub token: String,
    pub url: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub credential_id: Option<String>,
    pub principal_user_id: Option<String>,
    pub incoming_token_hash: Option<String>,
    pub signing_key_id: Option<String>,
    pub signing_ciphertext: Option<String>,
    pub credential_state: String,
    pub grant_version: i64,
    pub revoked_at: Option<String>,
    pub last_delivery_at: Option<String>,
    pub last_safe_error_code: Option<String>,
}

/// Parameters for creating a webhook (avoids too-many-arguments).
pub struct CreateWebhookParams<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub name: &'a str,
    pub avatar_url: Option<&'a str>,
    pub webhook_type: &'a str,
    pub token: &'a str,
    pub url: Option<&'a str>,
    pub created_by: &'a str,
}

/// A webhook event subscription.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookEventRow {
    pub id: String,
    pub webhook_id: String,
    pub event_type: String,
}

/// A slash command registered by a bot.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SlashCommandRow {
    pub id: String,
    pub bot_user_id: String,
    pub server_id: Option<String>,
    pub name: String,
    pub description: String,
    pub options_json: String,
    pub created_at: String,
}

/// An interaction (command invocation, button click, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InteractionRow {
    pub id: String,
    pub interaction_type: String,
    pub command_id: Option<String>,
    pub user_id: String,
    pub server_id: String,
    pub channel_id: String,
    pub data_json: String,
    pub responded: i32,
    pub created_at: String,
}

/// Parameters for creating an interaction (avoids too-many-arguments).
pub struct CreateInteractionParams<'a> {
    pub id: &'a str,
    pub interaction_type: &'a str,
    pub command_id: Option<&'a str>,
    pub user_id: &'a str,
    pub server_id: &'a str,
    pub channel_id: &'a str,
    pub data_json: &'a str,
    pub application_user_id: &'a str,
    pub expires_at: &'a str,
}

/// An OAuth2 application.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuth2AppRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub owner_id: String,
    pub client_secret: String,
    pub redirect_uris: String,
    pub scopes: String,
    pub is_public: i32,
    pub created_at: String,
}

/// An OAuth2 authorization grant.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuth2AuthorizationRow {
    pub id: String,
    pub app_id: String,
    pub user_id: String,
    pub server_id: Option<String>,
    pub scopes: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: String,
    pub created_at: String,
}

/// Parameters for creating a slash command (avoids too-many-arguments).
pub struct CreateSlashCommandParams<'a> {
    pub id: &'a str,
    pub bot_user_id: &'a str,
    pub server_id: Option<&'a str>,
    pub name: &'a str,
    pub description: &'a str,
    pub options_json: &'a str,
}

/// Parameters for creating an OAuth2 app (avoids too-many-arguments).
pub struct CreateOAuth2AppParams<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub icon_url: Option<&'a str>,
    pub owner_id: &'a str,
    pub client_secret: &'a str,
    pub redirect_uris: &'a str,
    pub scopes: &'a str,
    pub client_type: &'a str,
}

/// Parameters for creating an OAuth2 authorization (avoids too-many-arguments).
pub struct CreateOAuth2AuthParams<'a> {
    pub id: &'a str,
    pub app_id: &'a str,
    pub user_id: &'a str,
    pub server_id: Option<&'a str>,
    pub scopes: &'a str,
    pub access_token: &'a str,
    pub refresh_token: Option<&'a str>,
    pub expires_at: &'a str,
}
