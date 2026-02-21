use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
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
    let mut config = ServerConfig::load("concord.toml");

    // Reject the hardcoded default JWT secret in production
    const DEFAULT_SECRET: &str = "concord-dev-secret-change-me";
    if config.auth.jwt_secret == DEFAULT_SECRET || config.auth.jwt_secret.is_empty() {
        // Auto-generate a random 256-bit secret for this run
        use rand::Rng;
        let secret: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(44)
            .map(char::from)
            .collect();
        config.auth.jwt_secret = secret;
        warn!(
            "JWT secret is the default or empty — generated a random ephemeral secret. Sessions will NOT persist across restarts. Set jwt_secret in concord.toml or JWT_SECRET env var for production."
        );
    }

    // Initialize database
    let pool = create_pool(&config.database.url)
        .await
        .expect("failed to connect to database");

    run_migrations(&pool)
        .await
        .expect("failed to run database migrations");

    // Bootstrap admin users from config
    for username in &config.admin.admin_users {
        match sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&pool)
            .await
        {
            Ok(Some(user_id)) => {
                let _ = sqlx::query("UPDATE users SET is_system_admin = 1 WHERE id = ?")
                    .bind(&user_id)
                    .execute(&pool)
                    .await;
                info!(%username, "bootstrapped as system admin");
            }
            Ok(None) => {
                info!(%username, "admin user not found yet (will need manual promotion after first login)");
            }
            Err(e) => {
                tracing::warn!(%username, error = %e, "failed to bootstrap admin user");
            }
        }
    }

    // Create the shared chat engine with database
    let engine = Arc::new(ChatEngine::new(
        Some(pool.clone()),
        config.storage.max_message_length,
        config.storage.max_file_size_mb,
    ));

    // Load persisted servers and channels into memory
    engine
        .load_servers_from_db()
        .await
        .expect("failed to load servers from database");

    engine
        .load_channels_from_db()
        .await
        .expect("failed to load channels from database");

    // Spawn background task to periodically clean up stale rate-limiter buckets
    let engine_cleanup = engine.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            engine_cleanup.cleanup_rate_limiter();
            engine_cleanup.cleanup_slowmode_cache();
        }
    });

    // Cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    // Build optional TLS acceptor for IRC
    let irc_tls_acceptor = match (&config.server.irc_tls_cert, &config.server.irc_tls_key) {
        (Some(cert_path), Some(key_path)) => match load_irc_tls_config(cert_path, key_path) {
            Ok(acceptor) => {
                info!(
                    "IRC TLS configured with cert={}, key={}",
                    cert_path, key_path
                );
                Some(acceptor)
            }
            Err(e) => {
                panic!("Failed to load IRC TLS config: {e}");
            }
        },
        (Some(_), None) | (None, Some(_)) => {
            panic!("Both irc_tls_cert and irc_tls_key must be set for IRC TLS");
        }
        _ => None,
    };

    // Initialize IRC MOTD from config
    concord_server::irc::connection::set_motd(config.irc.motd.clone());

    // Start IRC listener
    let irc_engine = engine.clone();
    let irc_pool = pool.clone();
    let irc_addr = config.server.irc_address.clone();
    let irc_cancel = cancel.clone();
    tokio::spawn(async move {
        start_irc_listener(
            &irc_addr,
            irc_engine,
            irc_pool,
            irc_cancel,
            irc_tls_acceptor,
        )
        .await;
    });

    let max_file_size = config.storage.max_file_size_mb * 1024 * 1024;

    // Build shared app state for the web server
    let auth_config = config.to_auth_config();
    let atproto = AtprotoOAuth::load_or_create(&pool).await;
    let max_message_length = config.storage.max_message_length;
    let app_state = Arc::new(AppState {
        engine,
        db: pool,
        auth_config,
        atproto,
        max_file_size,
        max_message_length,
        jwt_blocklist: concord_server::auth::token::JwtBlocklist::new(),
    });

    // Periodically clean up expired entries from the JWT revocation blocklist
    let app_state_cleanup2 = app_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            app_state_cleanup2.jwt_blocklist.cleanup();
        }
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
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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

/// Load TLS certificate and private key from PEM files and build a TLS acceptor.
fn load_irc_tls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<tokio_rustls::TlsAcceptor, Box<dyn std::error::Error>> {
    use rustls_pemfile::{certs, private_key};
    use std::fs::File;
    use std::io::BufReader;
    use tokio_rustls::rustls::ServerConfig;

    let cert_file = File::open(cert_path)?;
    let key_file = File::open(key_path)?;

    let certs: Vec<_> = certs(&mut BufReader::new(cert_file)).collect::<Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        return Err("No certificates found in cert file".into());
    }

    let key =
        private_key(&mut BufReader::new(key_file))?.ok_or("No private key found in key file")?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}
