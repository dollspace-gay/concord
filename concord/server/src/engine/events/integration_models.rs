use super::{Deserialize, JsonSchema, Serialize};

/// Webhook info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebhookInfo {
    pub id: String,
    pub server_id: String,
    pub channel_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub webhook_type: String,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

/// Slash command info sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SlashCommandInfo {
    pub id: String,
    pub bot_user_id: String,
    pub name: String,
    pub description: String,
    pub options: Vec<SlashCommandOption>,
}

/// A single option/parameter for a slash command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SlashCommandOption {
    pub name: String,
    pub description: String,
    pub option_type: String, // "string", "integer", "boolean", "user", "channel", "role"
    #[serde(default)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<SlashCommandChoice>>,
}

/// A pre-defined choice for a slash command option.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SlashCommandChoice {
    pub name: String,
    pub value: String,
}

/// Interaction info sent to bots.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InteractionInfo {
    pub id: String,
    pub interaction_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_name: Option<String>,
    pub user_id: String,
    pub server_id: String,
    pub channel_id: String,
    pub data: serde_json::Value,
}

/// Bot's response to an interaction.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InteractionResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<RichEmbedInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<MessageComponent>>,
    #[serde(default)]
    pub ephemeral: bool,
}

/// Rich embed format for bot messages.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RichEmbedInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<EmbedField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// A field in a rich embed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

/// Footer for a rich embed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct EmbedFooter {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Author section for a rich embed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct EmbedAuthor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// A message component (button, select menu, or action row).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageComponent {
    ActionRow {
        components: Vec<MessageComponent>,
    },
    Button {
        custom_id: String,
        label: String,
        #[serde(default = "default_button_style")]
        style: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        emoji: Option<String>,
        #[serde(default)]
        disabled: bool,
    },
    SelectMenu {
        custom_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        options: Vec<SelectOption>,
        #[serde(default = "default_one")]
        min_values: i32,
        #[serde(default = "default_one")]
        max_values: i32,
    },
}

pub(super) fn default_button_style() -> String {
    "primary".to_string()
}

pub(super) fn default_one() -> i32 {
    1
}

/// An option in a select menu component.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default)]
    pub default: bool,
}

/// Bot token info (without the actual hash).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BotTokenInfo {
    pub id: String,
    pub name: String,
    pub scopes: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BotAccountInfo {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub installed_server_ids: Vec<String>,
}

/// OAuth2 application info.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OAuth2AppInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub owner_id: String,
    pub redirect_uris: Vec<String>,
    pub scopes: String,
    pub is_public: bool,
    pub created_at: String,
}
