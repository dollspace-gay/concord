/// Unique identifier for a message.
pub use crate::engine::ids::{ConnectionId, MessageId};

use chrono::{DateTime, Utc};

use schemars::JsonSchema;

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

mod community_models;
mod conversation_models;
mod integration_models;
mod message_models;
mod moderation_models;
mod organization_models;
mod protocol;
mod user_models;
pub use community_models::ChannelFollowInfo;
pub use community_models::EventInfo;
pub use community_models::InviteInfo;
pub use community_models::RsvpInfo;
pub use community_models::ServerCommunityInfo;
pub use community_models::TemplateInfo;
pub use conversation_models::BookmarkInfo;
pub use conversation_models::DirectConversationInfo;
pub use conversation_models::ForumTagInfo;
pub use conversation_models::PinnedMessageInfo;
pub use conversation_models::ThreadInfo;
pub use integration_models::BotAccountInfo;
pub use integration_models::BotTokenInfo;
pub use integration_models::EmbedAuthor;
pub use integration_models::EmbedField;
pub use integration_models::EmbedFooter;
pub use integration_models::InteractionInfo;
pub use integration_models::InteractionResponseData;
pub use integration_models::MessageComponent;
pub use integration_models::OAuth2AppInfo;
pub use integration_models::RichEmbedInfo;
pub use integration_models::SelectOption;
pub use integration_models::SlashCommandChoice;
pub use integration_models::SlashCommandInfo;
pub use integration_models::SlashCommandOption;
pub use integration_models::WebhookInfo;
pub use message_models::AttachmentInfo;
pub use message_models::ChannelInfo;
pub use message_models::EmbedInfo;
pub use message_models::HistoryMessage;
pub use message_models::MemberInfo;
pub use message_models::MemberRoleInfo;
pub use message_models::ReactionGroup;
pub use message_models::ReplyInfo;
pub use message_models::ServerInfo;
pub use message_models::UnreadCount;
pub use moderation_models::AuditLogEntry;
pub use moderation_models::AutomodRuleInfo;
pub use moderation_models::BanInfo;
pub use organization_models::CategoryInfo;
pub use organization_models::ChannelPermissionOverrideInfo;
pub use organization_models::ChannelPositionInfo;
pub use organization_models::RoleInfo;
pub use protocol::ChatEvent;
pub use user_models::NotificationSettingInfo;
pub use user_models::PresenceInfo;
pub use user_models::SearchResultMessage;
pub use user_models::UserProfileInfo;
