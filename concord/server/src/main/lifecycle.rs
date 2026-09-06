use super::{Arc, Duration, JoinSet, Result, anyhow};
use anyhow::Context;

pub(super) async fn bind_required_listener(
    name: &'static str,
    address: &str,
) -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind(address)
        .await
        .unwrap_or_else(|error| {
            eprintln!("failed to bind required {name} listener at {address}: {error}");
            std::process::exit(1);
        })
}

pub(super) fn unexpected_task_exit(
    result: Option<std::result::Result<(&'static str, Result<()>), tokio::task::JoinError>>,
) -> anyhow::Error {
    match result {
        Some(Ok((name, Ok(())))) => anyhow!("required task {name} exited unexpectedly"),
        Some(Ok((name, Err(error)))) => error.context(format!("required task {name} failed")),
        Some(Err(error)) => anyhow!("supervised task panicked or was cancelled: {error}"),
        None => anyhow!("supervisor has no running tasks"),
    }
}

pub(super) async fn drain_tasks(
    tasks: &mut JoinSet<(&'static str, Result<()>)>,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while !tasks.is_empty() {
        let result = match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(result) => result,
            Err(_) => {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err(anyhow!(
                    "shutdown exceeded the configured {timeout:?} deadline"
                ));
            }
        };
        match result {
            Some(Ok((_name, Ok(())))) => {}
            Some(Ok((name, Err(error)))) => {
                return Err(error.context(format!("supervised task {name} failed during shutdown")));
            }
            Some(Err(error)) if error.is_cancelled() => {}
            Some(Err(error)) => return Err(anyhow!("supervised task panicked: {error}")),
            None => break,
        }
    }
    Ok(())
}

pub(super) async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("register SIGTERM handler")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("register SIGINT handler"),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("register Ctrl+C handler")
    }
}

/// Load TLS certificate and private key from PEM files and build a TLS acceptor.
pub(super) fn load_irc_tls_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> std::result::Result<tokio_rustls::TlsAcceptor, Box<dyn std::error::Error>> {
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
    use tokio_rustls::rustls::{ServerConfig, crypto::ring};

    let certs: Vec<_> = CertificateDer::pem_file_iter(cert_path)?.collect::<Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        return Err("No certificates found in cert file".into());
    }

    let key = PrivateKeyDer::from_pem_file(key_path)?;

    // The dependency graph contains both Rustls providers (Reqwest selects
    // ring while Tokio Rustls defaults to aws-lc-rs), so the process-wide
    // implicit provider is intentionally unavailable. Select ring for this
    // listener explicitly instead of allowing TLS startup to panic.
    let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}
