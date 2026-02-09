use std::sync::Arc;

use sqlx::SqlitePool;

use crate::auth::config::AuthConfig;
use crate::engine::chat_engine::ChatEngine;

use super::atproto::AtprotoOAuth;

/// Shared application state available to all HTTP/WebSocket handlers.
pub struct AppState {
    pub engine: Arc<ChatEngine>,
    pub db: SqlitePool,
    pub auth_config: AuthConfig,
    pub atproto: AtprotoOAuth,
}
