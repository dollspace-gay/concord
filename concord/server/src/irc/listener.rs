use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::auth::authority::AuthService;
use crate::engine::chat_engine::ChatEngine;

use super::connection::handle_irc_connection_until;

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
    auth: AuthService,
    cancel: CancellationToken,
    tls_acceptor: Option<TlsAcceptor>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    run_irc_listener(listener, engine, db, auth, cancel, tls_acceptor).await
}

/// Run IRC using an already-bound listener so startup can establish all required sockets
/// before the service reports readiness.
pub async fn run_irc_listener(
    listener: TcpListener,
    engine: Arc<ChatEngine>,
    db: SqlitePool,
    auth: AuthService,
    cancel: CancellationToken,
    tls_acceptor: Option<TlsAcceptor>,
) -> std::io::Result<()> {
    let bind_addr = listener.local_addr()?;

    if tls_acceptor.is_some() {
        info!("IRC listener started on {} (TLS enabled)", bind_addr);
    } else {
        info!("IRC listener started on {} (plaintext)", bind_addr);
    }

    // Track active connection count per IP
    let ip_counts: Arc<DashMap<IpAddr, AtomicU32>> = Arc::new(DashMap::new());
    let mut connections = JoinSet::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("IRC listener shutting down");
                break;
            }
            result = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = result {
                    warn!(%error, "IRC connection task failed");
                }
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
                        let auth = auth.clone();
                        let peer = addr.to_string();
                        let ip_counts = ip_counts.clone();
                        let connection_cancel = cancel.child_token();
                        let count_guard = ConnectionCountGuard::new(ip_counts, ip);
                        if let Some(ref acceptor) = tls_acceptor {
                            let acceptor = acceptor.clone();
                            connections.spawn(async move {
                                let _count_guard = count_guard;
                                let handshake = tokio::select! {
                                    _ = connection_cancel.cancelled() => return,
                                    result = tokio::time::timeout(
                                        TLS_HANDSHAKE_TIMEOUT,
                                        acceptor.accept(stream),
                                    ) => result,
                                };
                                match handshake {
                                    Ok(Ok(tls_stream)) => {
                                        handle_irc_connection_until(
                                            tls_stream,
                                            peer,
                                            engine,
                                            db,
                                            auth,
                                            connection_cancel,
                                        )
                                        .await;
                                    }
                                    Ok(Err(e)) => {
                                        warn!(%peer, error = %e, "TLS handshake failed");
                                    }
                                    Err(_) => {
                                        warn!(%peer, "TLS handshake timed out");
                                    }
                                }
                            });
                        } else {
                            connections.spawn(async move {
                                let _count_guard = count_guard;
                                handle_irc_connection_until(
                                    stream,
                                    peer,
                                    engine,
                                    db,
                                    auth,
                                    connection_cancel,
                                )
                                .await;
                            });
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }

    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            warn!(%error, "IRC connection task failed during shutdown");
        }
    }
    Ok(())
}

struct ConnectionCountGuard {
    counts: Arc<DashMap<IpAddr, AtomicU32>>,
    ip: IpAddr,
}

impl ConnectionCountGuard {
    fn new(counts: Arc<DashMap<IpAddr, AtomicU32>>, ip: IpAddr) -> Self {
        Self { counts, ip }
    }
}

impl Drop for ConnectionCountGuard {
    fn drop(&mut self) {
        if let dashmap::mapref::entry::Entry::Occupied(entry) = self.counts.entry(self.ip) {
            if entry.get().load(Ordering::Relaxed) <= 1 {
                entry.remove();
            } else {
                entry.get().fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_one_guard_preserves_a_concurrent_connection_count() {
        let counts = Arc::new(DashMap::new());
        let ip = "127.0.0.1".parse().unwrap();
        counts.insert(ip, AtomicU32::new(1));
        let guard = ConnectionCountGuard::new(counts.clone(), ip);

        counts
            .entry(ip)
            .or_insert_with(|| AtomicU32::new(0))
            .fetch_add(1, Ordering::Relaxed);
        drop(guard);

        assert_eq!(counts.get(&ip).unwrap().load(Ordering::Relaxed), 1);
    }
}
