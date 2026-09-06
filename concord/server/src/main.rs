use std::sync::Arc;

use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use tokio::task::JoinSet;

use tokio_util::sync::CancellationToken;

use tracing::{info, warn};

use tracing_subscriber::EnvFilter;

use concord_server::config::ServerConfig;

use concord_server::db::pool::{create_pool, run_migrations};

use concord_server::engine::chat_engine::ChatEngine;

use concord_server::irc::listener::run_irc_listener;

use concord_server::web::app_state::{AppState, HealthState};

use concord_server::web::atproto::AtprotoOAuth;

use concord_server::web::router::build_router;

fn configured_egress(config: &ServerConfig) -> Result<concord_server::egress::EgressServices> {
    #[cfg(feature = "browser-fixtures")]
    if let Some(raw_address) = std::env::var_os("CONCORD_BROWSER_EGRESS_FIXTURE_ADDR") {
        let address: std::net::SocketAddr = raw_address
            .to_str()
            .ok_or_else(|| anyhow!("browser egress fixture address is not valid UTF-8"))?
            .parse()
            .context("browser egress fixture address is invalid")?;
        if !address.ip().is_loopback() {
            return Err(anyhow!(
                "browser egress fixture must use a loopback address"
            ));
        }
        return Ok(concord_server::egress::EgressServices::profile_fixture(
            address,
        ));
    }

    concord_server::egress::EgressServices::internet_with_admin_origins(
        &config.egress.operator_allowed_origins,
    )
    .map_err(Into::into)
}

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let (command, config_path) = parse_arguments().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    if command == Command::Init {
        ServerConfig::initialize(&config_path).unwrap_or_else(|error| {
            eprintln!("configuration initialization failed: {error}");
            std::process::exit(2);
        });
        println!("initialized {}", config_path.display());
        return;
    }
    let config = ServerConfig::load(&config_path).unwrap_or_else(|error| {
        eprintln!("configuration validation failed: {error}");
        std::process::exit(2);
    });
    if command == Command::ValidateConfig {
        println!("configuration is valid: {}", config_path.display());
        return;
    }
    let _maintenance_lock =
        concord_server::operations::acquire_database_exclusion(&config.database.url)
            .expect("failed to acquire service maintenance lock");
    concord_server::operations::ensure_restore_is_not_pending(&config.database.url)
        .expect("refusing to start during incomplete restore activation");

    // Initialize database
    let pool = create_pool(&config.database.url)
        .await
        .expect("failed to connect to database");

    run_migrations(&pool)
        .await
        .expect("failed to run database migrations");

    let auth_config = config.to_auth_config();
    let auth_service = concord_server::auth::authority::AuthService::new(
        pool.clone(),
        auth_config.jwt_secret.clone(),
        auth_config.session_expiry_hours,
    );

    let egress = Arc::new(
        configured_egress(&config).expect("failed to initialize controlled outbound transports"),
    );
    // Create the shared chat engine with mandatory durable storage and authority.
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth_service.clone(),
        &auth_config.jwt_secret,
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

    let engine_cleanup = engine.clone();
    let cancel = CancellationToken::new();

    // Build optional TLS acceptor for IRC
    let irc_tls_acceptor = match (&config.server.irc_tls_cert, &config.server.irc_tls_key) {
        (Some(cert_path), Some(key_path)) => match load_irc_tls_config(cert_path, key_path) {
            Ok(acceptor) => {
                info!(
                    "IRC TLS configured with cert={}, key={}",
                    cert_path.display(),
                    key_path.display()
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

    let irc_listener = bind_required_listener("IRC", &config.server.irc_address).await;
    let web_listener = bind_required_listener("web", &config.server.web_address).await;

    let max_file_size = config.storage.max_file_size_mb * 1024 * 1024;

    // Build shared app state for the web server
    let secret_vault = Arc::new(
        concord_server::secrets::SecretVault::load(&config.auth.external_credentials_key_file)
            .unwrap_or_else(|error| {
                eprintln!("external credential key validation failed: {error}");
                std::process::exit(2);
            }),
    );
    engine
        .configure_integration_vault(secret_vault.clone())
        .expect("integration vault must be configured once");
    let atproto = match AtprotoOAuth::load_or_create(&pool, &secret_vault).await {
        Ok(value) => value,
        Err(error) => {
            warn!(error=%error,"AT Protocol integration disabled because signing key recovery failed");
            AtprotoOAuth::unavailable()
        }
    };
    let max_message_length = config.storage.max_message_length;
    let health = Arc::new(HealthState::default());
    health.set_irc_listener_bound(true);
    health.set_web_listener_bound(true);
    let app_state = Arc::new(AppState {
        engine: engine.clone(),
        db: pool.clone(),
        auth_config,
        auth: auth_service.clone(),
        atproto,
        secret_vault,
        egress,
        max_file_size,
        max_media_per_user: config.storage.max_media_per_user_mb * 1024 * 1024,
        max_media_total: config.storage.max_media_total_mb * 1024 * 1024,
        upload_admission: Arc::new(tokio::sync::Semaphore::new(4)),
        upload_idle_timeout: Duration::from_secs(15),
        upload_total_timeout: Duration::from_secs(300),
        max_message_length,
        admin_user_ids: config.admin.admin_user_ids.clone().into(),
        health: health.clone(),
        shutdown: cancel.clone(),
        media_dir: config.storage.media_dir.clone(),
    });

    let app = build_router(app_state.clone());

    info!(
        "Concord server starting — Web: {}, IRC: {}",
        config.server.web_address, config.server.irc_address
    );

    let mut tasks: JoinSet<(&'static str, Result<()>)> = JoinSet::new();
    let dispatcher_engine = engine.clone();
    let dispatcher_cancel = cancel.child_token();
    tasks.spawn(async move {
        let result = dispatcher_engine
            .run_delivery_dispatcher(dispatcher_cancel)
            .await
            .map_err(anyhow::Error::msg)
            .context("durable delivery dispatcher failed");
        ("durable delivery dispatcher", result)
    });
    let irc_cancel = cancel.child_token();
    tasks.spawn(async move {
        let result = run_irc_listener(
            irc_listener,
            engine,
            pool,
            auth_service,
            irc_cancel,
            irc_tls_acceptor,
        )
        .await
        .context("IRC listener failed");
        ("IRC listener", result)
    });
    let web_cancel = cancel.child_token();
    tasks.spawn(async move {
        let result = axum::serve(
            web_listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move { web_cancel.cancelled().await })
        .await
        .context("web listener failed");
        ("web listener", result)
    });
    let cleanup_cancel = cancel.child_token();
    tasks.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = cleanup_cancel.cancelled() => break,
                _ = interval.tick() => {
                    engine_cleanup.cleanup_rate_limiter();
                    engine_cleanup.cleanup_slowmode_cache();
                }
            }
        }
        ("cache maintenance", Ok(()))
    });
    let media_cancel = cancel.child_token();
    let media_pool = app_state.db.clone();
    let media_root = app_state.media_dir.clone();
    tasks.spawn(async move {
        let result: Result<()> = async {
            concord_server::media::reconcile_interrupted(&media_pool, &media_root)
                .await
                .context("interrupted media reconciliation failed")?;
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                tokio::select! {
                    _ = media_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        concord_server::media::collect_expired(&media_pool,&media_root,3600).await
                            .context("staging media collection failed")?;
                        concord_server::media::collect_deleted(&media_pool,&media_root).await
                            .context("deleted media collection failed")?;
                    }
                }
            }
            Ok(())
        }
        .await;
        ("media maintenance", result)
    });
    if let Some(signing_key) = app_state.atproto.signing_key.clone() {
        let publication_dispatcher =
            concord_server::web::atproto_records::AtprotoPublicationDispatcher::new(
                app_state.db.clone(),
                app_state.egress.oauth.clone(),
                app_state.secret_vault.clone(),
                Arc::new(signing_key),
                format!(
                    "{}/api/auth/atproto/v2/client-metadata.json",
                    app_state.auth_config.public_url
                ),
                format!(
                    "{}/api/auth/atproto/callback",
                    app_state.auth_config.public_url
                ),
            );
        let publication_cancel = cancel.child_token();
        let publication_pool = app_state.db.clone();
        tasks.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = publication_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        if let Err(error) = concord_server::jobs::run_once_matching(
                            &publication_pool,
                            "atproto-publication-worker",
                            &publication_dispatcher,
                            &concord_server::jobs::JobSelection {
                                operation_types: &["atproto_publish", "atproto_update", "atproto_delete"],
                                lease_seconds: 60,
                                limit: 10,
                                max_attempts: 12,
                            },
                        ).await {
                            warn!(%error, "AT publication worker iteration failed");
                        }
                    }
                }
            }
            ("AT publication worker", Ok(()))
        });
    }
    let webhook_dispatcher = concord_server::web::webhook_dispatcher::WebhookDispatcher::new(
        app_state.db.clone(),
        app_state.egress.general.clone(),
        app_state.secret_vault.clone(),
        8,
    );
    let webhook_cancel = cancel.child_token();
    let webhook_pool = app_state.db.clone();
    tasks.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = webhook_cancel.cancelled() => break,
                _ = interval.tick() => {
                    if let Err(error) = concord_server::jobs::run_once_matching(
                        &webhook_pool,
                        "webhook-delivery-worker",
                        &webhook_dispatcher,
                        &concord_server::jobs::JobSelection {
                            operation_types: &["webhook_delivery"],
                            lease_seconds: 60,
                            // A single in-flight delivery cannot expire behind a slow predecessor.
                            limit: 1,
                            max_attempts: 8,
                        },
                    ).await {
                        warn!(%error, "webhook delivery worker iteration failed");
                    }
                }
            }
        }
        ("webhook delivery worker", Ok(()))
    });

    health.set_ready(true);
    let early_failure = tokio::select! {
        signal = shutdown_signal() => {
            if signal.is_ok() {
                info!("shutdown signal received");
            }
            signal.err()
        }
        result = tasks.join_next() => Some(unexpected_task_exit(result)),
    };
    health.set_ready(false);
    cancel.cancel();

    let drain_result = drain_tasks(
        &mut tasks,
        Duration::from_secs(config.server.shutdown_timeout_seconds),
    )
    .await;
    if let Some(error) = early_failure.into_iter().chain(drain_result.err()).next() {
        tracing::error!(%error, "Concord stopped after a supervised task failure");
        std::process::exit(1);
    }
    info!("Concord server stopped");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Serve,
    Init,
    ValidateConfig,
}

fn parse_arguments() -> Result<(Command, std::path::PathBuf)> {
    let mut command = None;
    let mut config_path = std::env::var_os("CONCORD_CONFIG").map(std::path::PathBuf::from);
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--config" {
            config_path = Some(
                arguments
                    .next()
                    .map(std::path::PathBuf::from)
                    .context("--config requires a path")?,
            );
            continue;
        }
        let argument = argument
            .to_str()
            .context("command arguments must be valid UTF-8")?;
        let parsed = match argument {
            "serve" => Command::Serve,
            "init" => Command::Init,
            "validate-config" => Command::ValidateConfig,
            "--help" | "-h" => {
                println!(
                    "Usage: concord-server [serve|init|validate-config] [--config PATH]\n\
                     CONCORD_CONFIG supplies the path when --config is omitted."
                );
                std::process::exit(0);
            }
            _ => return Err(anyhow!("unknown argument: {argument}")),
        };
        if command.replace(parsed).is_some() {
            return Err(anyhow!("only one command may be selected"));
        }
    }
    Ok((
        command.unwrap_or(Command::Serve),
        config_path.unwrap_or_else(|| "concord.toml".into()),
    ))
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;

#[path = "main/lifecycle.rs"]
mod lifecycle;
use lifecycle::bind_required_listener;
use lifecycle::drain_tasks;
use lifecycle::load_irc_tls_config;
use lifecycle::shutdown_signal;
use lifecycle::unexpected_task_exit;
