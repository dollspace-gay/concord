use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use super::events::{ChatEvent, SessionId};

/// Maximum queued outbound events per session (prevents memory exhaustion from slow clients).
pub const MAX_OUTBOUND_QUEUE: usize = 1024;

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
    /// Send outbound events to this session's write loop (bounded to prevent memory exhaustion).
    pub outbound: mpsc::Sender<ChatEvent>,
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
        outbound: mpsc::Sender<ChatEvent>,
        avatar_url: Option<String>,
    ) -> Self {
        Self {
            id,
            user_id,
            nickname,
            protocol,
            outbound,
            connected_at: Utc::now(),
            avatar_url,
        }
    }

    /// Send an event to this session. Returns false if the channel is closed
    /// or the outbound queue is full (slow client protection — drops event rather than blocking).
    pub fn send(&self, event: ChatEvent) -> bool {
        self.outbound.try_send(event).is_ok()
    }
}
