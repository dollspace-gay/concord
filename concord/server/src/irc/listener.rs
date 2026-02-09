use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::engine::chat_engine::ChatEngine;

use super::connection::handle_irc_connection;

/// Start the IRC TCP listener. Accepts connections and spawns a handler task for each.
/// Stops accepting new connections when the cancellation token is triggered.
pub async fn start_irc_listener(
    bind_addr: &str,
    engine: Arc<ChatEngine>,
    db: SqlitePool,
    cancel: CancellationToken,
) {
    let listener = TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind IRC listener");

    info!("IRC listener started on {}", bind_addr);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("IRC listener shutting down");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, _addr)) => {
                        let engine = engine.clone();
                        let db = db.clone();
                        tokio::spawn(async move {
                            handle_irc_connection(stream, engine, db).await;
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "failed to accept IRC connection");
                    }
                }
            }
        }
    }
}
