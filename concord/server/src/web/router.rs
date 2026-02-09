use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::engine::chat_engine::ChatEngine;

use super::ws_handler::ws_upgrade;

/// Build the axum router with all HTTP and WebSocket routes.
pub fn build_router(engine: Arc<ChatEngine>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/ws", axum::routing::get(ws_upgrade))
        .fallback_service(ServeDir::new("static"))
        .layer(cors)
        .with_state(engine)
}
