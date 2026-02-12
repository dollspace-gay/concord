use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::engine::chat_engine::ChatEngine;

use super::connection::handle_irc_connection;

/// Start the IRC TCP listener. Accepts connections and spawns a handler task for each.
/// If a TLS acceptor is provided, connections are wrapped in TLS.
/// Stops accepting new connections when the cancellation token is triggered.
pub async fn start_irc_listener(
    bind_addr: &str,
    engine: Arc<ChatEngine>,
    db: SqlitePool,
    cancel: CancellationToken,
    tls_acceptor: Option<TlsAcceptor>,
) {
    let listener = TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind IRC listener");

    if tls_acceptor.is_some() {
        info!("IRC listener started on {} (TLS enabled)", bind_addr);
    } else {
        info!("IRC listener started on {} (plaintext)", bind_addr);
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("IRC listener shutting down");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let engine = engine.clone();
                        let db = db.clone();
                        let peer = addr.to_string();
                        if let Some(ref acceptor) = tls_acceptor {
                            let acceptor = acceptor.clone();
                            tokio::spawn(async move {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => {
                                        handle_irc_connection(tls_stream, peer, engine, db).await;
                                    }
                                    Err(e) => {
                                        warn!(%peer, error = %e, "TLS handshake failed");
                                    }
                                }
                            });
                        } else {
                            tokio::spawn(async move {
                                handle_irc_connection(stream, peer, engine, db).await;
                            });
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "failed to accept IRC connection");
                    }
                }
            }
        }
    }
}
