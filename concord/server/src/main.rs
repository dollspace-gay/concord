use std::sync::Arc;

use tracing::info;
use tracing_subscriber::EnvFilter;

use concord_server::engine::chat_engine::ChatEngine;
use concord_server::web::router::build_router;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind_addr = "0.0.0.0:8080";

    // Create the shared chat engine
    let engine = Arc::new(ChatEngine::new());

    // Build the HTTP/WebSocket router
    let app = build_router(engine);

    info!("Concord server starting on {}", bind_addr);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind to address");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
