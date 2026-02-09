use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::events::SessionId;

/// In-memory state for a single channel.
#[derive(Debug)]
pub struct ChannelState {
    pub name: String,
    pub topic: String,
    pub topic_set_by: Option<String>,
    pub topic_set_at: Option<DateTime<Utc>>,
    /// Session IDs of currently connected members.
    pub members: HashSet<SessionId>,
    pub created_at: DateTime<Utc>,
}

impl ChannelState {
    pub fn new(name: String) -> Self {
        Self {
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
