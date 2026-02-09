use std::collections::HashSet;

/// In-memory state for a server (guild).
#[derive(Debug)]
pub struct ServerState {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub owner_id: String,
    /// Channel IDs belonging to this server.
    pub channel_ids: HashSet<String>,
    /// User IDs who are members of this server (persistent membership).
    pub member_user_ids: HashSet<String>,
}

impl ServerState {
    pub fn new(id: String, name: String, owner_id: String, icon_url: Option<String>) -> Self {
        Self {
            id,
            name,
            icon_url,
            owner_id,
            channel_ids: HashSet::new(),
            member_user_ids: HashSet::new(),
        }
    }
}
