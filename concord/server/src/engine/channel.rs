use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::events::SessionId;

/// In-memory state for a single channel.
#[derive(Debug)]
pub struct ChannelState {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub topic: String,
    pub topic_set_by: Option<String>,
    pub topic_set_at: Option<DateTime<Utc>>,
    /// Session IDs of currently connected members.
    pub members: HashSet<SessionId>,
    pub created_at: DateTime<Utc>,
}

impl ChannelState {
    pub fn new(id: String, server_id: String, name: String) -> Self {
        Self {
            id,
            server_id,
            name,
            topic: String::new(),
            topic_set_by: None,
            topic_set_at: None,
            members: HashSet::new(),
            created_at: Utc::now(),
        }
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }
}
