use std::sync::Arc;

use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};

use axum::extract::{Extension, State, WebSocketUpgrade};

use axum::http::HeaderMap;

use axum::response::IntoResponse;

use axum_extra::extract::CookieJar;

use futures_util::{SinkExt, StreamExt};

use schemars::JsonSchema;

use serde::{Deserialize, Serialize};

use tracing::{error, info, warn};

use crate::auth::authority::{Actor, AuthService};

use crate::db::queries::users;

use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};

use crate::engine::events::ChatEvent;

use crate::engine::permissions::Permissions;

use crate::engine::user_session::Protocol;

use super::app_state::AppState;

use command_policy::fixed_window_admit;
use command_policy::websocket_command_correlation;
use command_policy::websocket_command_is_read;

pub use connection::ws_upgrade;
use envelope::handle_client_message;
use errors::lifecycle_command_allowed;
use errors::send_error;
use errors::split_safe_error;
pub(crate) use protocol::ClientMessage;

mod announcements;
mod automod;
mod bookmarks;
mod bots;
mod categories;
mod channel_lifecycle;
mod channel_permissions;
mod channels;
mod command_policy;
mod community;
mod connection;
mod dispatch;
mod envelope;
mod errors;
mod events;
mod forum_tags;
mod interactions;
mod invites;
mod message_mutations;
mod messaging;
mod moderation;
mod notifications;
mod oauth_apps;
mod pins;
mod presence;
mod profiles;
mod protocol;
mod read_state;
mod roles;
mod search;
mod server_management;
mod server_profile;
mod servers;
mod slash_commands;
mod synchronization;
mod templates;
mod threads;
mod webhooks;

#[cfg(test)]
mod tests;
