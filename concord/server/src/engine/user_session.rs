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
    pub nickname: String,
    pub protocol: Protocol,
    /// Send outbound events to this session's write loop.
    pub outbound: mpsc::UnboundedSender<ChatEvent>,
    /// Channels this session is currently in.
    pub channels: HashSet<String>,
    pub connected_at: DateTime<Utc>,
}

impl UserSession {
    pub fn new(
        id: SessionId,
        nickname: String,
        protocol: Protocol,
        outbound: mpsc::UnboundedSender<ChatEvent>,
    ) -> Self {
        Self {
            id,
            nickname,
            protocol,
            outbound,
            channels: HashSet::new(),
            connected_at: Utc::now(),
        }
    }

    /// Send an event to this session. Returns false if the channel is closed.
    pub fn send(&self, event: ChatEvent) -> bool {
        self.outbound.send(event).is_ok()
    }
}
