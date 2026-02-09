use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

use concord_server::config::ServerConfig;
use concord_server::db::pool::{create_pool, run_migrations};
use concord_server::engine::chat_engine::ChatEngine;
use concord_server::irc::listener::start_irc_listener;
use concord_server::web::app_state::AppState;
use concord_server::web::atproto::AtprotoOAuth;
use concord_server::web::router::build_router;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load configuration (TOML file + env overrides)
    let config = ServerConfig::load("concord.toml");

    // Initialize database
    let pool = create_pool(&config.database.url)
        .await
        .expect("failed to connect to database");

    run_migrations(&pool)
        .await
        .expect("failed to run database migrations");

    // Create the shared chat engine with database
    let engine = Arc::new(ChatEngine::new(Some(pool.clone())));

    // Load persisted channels into memory
    engine
        .load_channels_from_db()
        .await
        .expect("failed to load channels from database");

    // Cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    // Start IRC listener
    let irc_engine = engine.clone();
    let irc_pool = pool.clone();
    let irc_addr = config.server.irc_address.clone();
    let irc_cancel = cancel.clone();
    tokio::spawn(async move {
        start_irc_listener(&irc_addr, irc_engine, irc_pool, irc_cancel).await;
    });

    // Build shared app state for the web server
    let auth_config = config.to_auth_config();
    let atproto = AtprotoOAuth::new();
    let app_state = Arc::new(AppState {
        engine,
        db: pool,
        auth_config,
        atproto,
    });

    let app = build_router(app_state);

    info!(
        "Concord server starting — Web: {}, IRC: {}",
        config.server.web_address, config.server.irc_address
    );

    let listener = tokio::net::TcpListener::bind(&config.server.web_address)
        .await
        .expect("failed to bind web listener");

    // Serve with graceful shutdown on Ctrl+C
    let shutdown_cancel = cancel.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to listen for Ctrl+C");
            info!("Shutdown signal received, stopping gracefully...");
            shutdown_cancel.cancel();
        })
        .await
        .expect("server error");

    info!("Concord server stopped");
}
