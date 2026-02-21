use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::engine::chat_engine::ChatEngine;

use super::connection::handle_irc_connection;

/// Maximum concurrent IRC connections per IP address.
const MAX_CONNECTIONS_PER_IP: u32 = 5;

/// Timeout for TLS handshake — prevents malicious clients from holding connections indefinitely.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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

    // Track active connection count per IP
    let ip_counts: Arc<DashMap<IpAddr, AtomicU32>> = Arc::new(DashMap::new());

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("IRC listener shutting down");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let ip = addr.ip();

                        // Enforce per-IP connection limit
                        let count = ip_counts
                            .entry(ip)
                            .or_insert_with(|| AtomicU32::new(0));
                        let current = count.load(Ordering::Relaxed);
                        if current >= MAX_CONNECTIONS_PER_IP {
                            warn!(%ip, count = current, "IRC connection rejected: per-IP limit reached");
                            drop(stream);
                            continue;
                        }
                        count.fetch_add(1, Ordering::Relaxed);

                        let engine = engine.clone();
                        let db = db.clone();
                        let peer = addr.to_string();
                        let ip_counts = ip_counts.clone();
                        if let Some(ref acceptor) = tls_acceptor {
                            let acceptor = acceptor.clone();
                            tokio::spawn(async move {
                                match tokio::time::timeout(
                                    TLS_HANDSHAKE_TIMEOUT,
                                    acceptor.accept(stream),
                                )
                                .await
                                {
                                    Ok(Ok(tls_stream)) => {
                                        handle_irc_connection(tls_stream, peer, engine, db).await;
                                    }
                                    Ok(Err(e)) => {
                                        warn!(%peer, error = %e, "TLS handshake failed");
                                    }
                                    Err(_) => {
                                        warn!(%peer, "TLS handshake timed out");
                                    }
                                }
                                decrement_ip_count(&ip_counts, ip);
                            });
                        } else {
                            tokio::spawn(async move {
                                handle_irc_connection(stream, peer, engine, db).await;
                                decrement_ip_count(&ip_counts, ip);
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

/// Decrement the connection count for an IP, removing the entry when it reaches zero.
fn decrement_ip_count(ip_counts: &DashMap<IpAddr, AtomicU32>, ip: IpAddr) {
    if let Some(entry) = ip_counts.get(&ip) {
        let prev = entry.fetch_sub(1, Ordering::Relaxed);
        drop(entry);
        if prev <= 1 {
            ip_counts.remove(&ip);
        }
    }
}
