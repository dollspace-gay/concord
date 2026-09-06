use super::{Deserialize, Serialize};

/// A server invite.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InviteRow {
    pub id: String,
    pub server_id: String,
    pub code: String,
    pub created_by: String,
    pub max_uses: Option<i32>,
    pub use_count: i32,
    pub expires_at: Option<String>,
    pub channel_id: Option<String>,
    pub created_at: String,
}

/// A scheduled server event.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerEventRow {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub description: Option<String>,
    pub channel_id: Option<String>,
    pub start_time: String,
    pub end_time: Option<String>,
    pub image_url: Option<String>,
    pub created_by: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// An event RSVP record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventRsvpRow {
    pub event_id: String,
    pub user_id: String,
    pub status: String,
    pub created_at: String,
}

/// A channel follow (for announcement cross-posting).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelFollowRow {
    pub id: String,
    pub source_channel_id: String,
    pub target_channel_id: String,
    pub created_by: String,
    pub created_at: String,
}

/// A server template.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerTemplateRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub created_by: String,
    pub config: String,
    pub use_count: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// Parameters for creating a server event (avoids too-many-arguments).
pub struct CreateServerEventParams<'a> {
    pub id: &'a str,
    pub server_id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub channel_id: Option<&'a str>,
    pub start_time: &'a str,
    pub end_time: Option<&'a str>,
    pub image_url: Option<&'a str>,
    pub created_by: &'a str,
}
