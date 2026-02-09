use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::engine::chat_engine::ChatEngine;

use super::connection::handle_irc_connection;

/// Start the IRC TCP listener. Accepts connections and spawns a handler task for each.
pub async fn start_irc_listener(bind_addr: &str, engine: Arc<ChatEngine>, db: SqlitePool) {
    let listener = TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind IRC listener");

    info!("IRC listener started on {}", bind_addr);

    loop {
        match listener.accept().await {
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
