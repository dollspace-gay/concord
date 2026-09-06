use super::{
    DateTime, Deserialize, JsonSchema, MessageComponent, MessageId, RichEmbedInfo, Serialize, Utc,
};

/// Info about a replied-to message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReplyInfo {
    pub id: String,
    pub from: String,
    pub content_preview: String,
}

/// Grouped reactions for a message in history.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: usize,
    pub user_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub member_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Effective permission bitfield for the requesting user in this server.
    #[serde(default)]
    pub my_permissions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelInfo {
    pub id: String,
    pub conversation_id: String,
    pub server_id: String,
    pub name: String,
    pub topic: String,
    pub member_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub position: i32,
    pub is_private: bool,
    pub channel_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_parent_message_id: Option<String>,
    pub archived: bool,
    pub slowmode_seconds: i32,
    pub is_nsfw: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemberInfo {
    pub nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub role_ids: Vec<String>,
}

/// One member's role assignments in an authoritative role bootstrap.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct MemberRoleInfo {
    pub user_id: String,
    pub role_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HistoryMessage {
    pub id: MessageId,
    pub from: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reactions: Option<Vec<ReactionGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<AttachmentInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<EmbedInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_embeds: Option<Vec<RichEmbedInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<MessageComponent>>,
}

/// Metadata for a file attachment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentInfo {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub url: String,
}

/// Open Graph link embed preview metadata.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EmbedInfo {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnreadCount {
    pub channel_name: String,
    pub count: i64,
}
