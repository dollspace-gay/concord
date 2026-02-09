use std::collections::HashSet;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use super::events::{ChatEvent, SessionId};

/// Which protocol this session connected via.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Irc,
    WebSocket,
}

/// A connected user session. Protocol-agnostic — the engine doesn't care
/// whether this is an IRC client or a web browser.
#[derive(Debug)]
pub struct UserSession {
    pub id: SessionId,
    /// Database user ID (None for unauthenticated/guest sessions).
    pub user_id: Option<String>,
    pub nickname: String,
    pub protocol: Protocol,
    /// Send outbound events to this session's write loop.
    pub outbound: mpsc::UnboundedSender<ChatEvent>,
    /// Channels this session is currently in.
    pub channels: HashSet<String>,
    pub connected_at: DateTime<Utc>,
    /// Avatar URL (from Bluesky profile or other source).
    pub avatar_url: Option<String>,
}

impl UserSession {
    pub fn new(
        id: SessionId,
        user_id: Option<String>,
        nickname: String,
        protocol: Protocol,
        outbound: mpsc::UnboundedSender<ChatEvent>,
        avatar_url: Option<String>,
    ) -> Self {
        Self {
            id,
            user_id,
            nickname,
            protocol,
            outbound,
            channels: HashSet::new(),
            connected_at: Utc::now(),
            avatar_url,
        }
    }

    /// Send an event to this session. Returns false if the channel is closed.
    pub fn send(&self, event: ChatEvent) -> bool {
        self.outbound.send(event).is_ok()
    }
}
