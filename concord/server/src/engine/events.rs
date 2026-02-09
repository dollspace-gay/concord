use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a message.
pub type MessageId = Uuid;

/// Unique identifier for a connected session (one per connection, not per user).
pub type SessionId = Uuid;

/// Protocol-agnostic event that flows through the chat engine.
/// Both IRC and WebSocket adapters produce and consume these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// A message sent to a channel or as a DM.
    Message {
        id: MessageId,
        from: String,
        target: String,
        content: String,
        timestamp: DateTime<Utc>,
    },

    /// User joined a channel.
    Join { nickname: String, channel: String },

    /// User left a channel.
    Part {
        nickname: String,
        channel: String,
        reason: Option<String>,
    },

    /// User disconnected from the server.
    Quit {
        nickname: String,
        reason: Option<String>,
    },

    /// Channel topic changed.
    TopicChange {
        channel: String,
        set_by: String,
        topic: String,
    },

    /// User changed their nickname.
    NickChange {
        old_nick: String,
        new_nick: String,
    },

    /// Server notice directed at a specific session.
    ServerNotice { message: String },

    /// Channel member list (sent on join).
    Names {
        channel: String,
        members: Vec<String>,
    },

    /// Current topic of a channel (sent on join).
    Topic {
        channel: String,
        topic: String,
    },

    /// Response to a channel list request.
    ChannelList {
        channels: Vec<ChannelInfo>,
    },

    /// Message history response.
    History {
        channel: String,
        messages: Vec<HistoryMessage>,
        has_more: bool,
    },

    /// Error from the server.
    Error { code: String, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub name: String,
    pub topic: String,
    pub member_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub id: MessageId,
    pub from: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}
