use super::{
    Arc, AsyncWrite, AsyncWriteExt, AuthService, CancellationToken, ChatEngine, Duration,
    OutboundLine, mpsc, sanitize_outbound_line,
};

pub(super) struct Writer {
    pub failed: CancellationToken,
    pub authority_failed: CancellationToken,
    pub cancel: CancellationToken,
    pub auth: AuthService,
    pub engine: Arc<ChatEngine>,
    pub outbound_actor: Arc<std::sync::RwLock<Option<crate::auth::authority::Actor>>>,
    pub queued_bytes: Arc<std::sync::atomic::AtomicUsize>,
}

impl Writer {
    pub async fn run<W: AsyncWrite + Unpin>(
        self,
        mut writer: W,
        mut out_rx: mpsc::Receiver<OutboundLine>,
    ) {
        let Self {
            failed,
            authority_failed,
            cancel,
            auth,
            engine,
            outbound_actor,
            queued_bytes,
        } = self;

        loop {
            let outbound = tokio::select! {
                _ = cancel.cancelled() => break,
                _ = failed.cancelled() => break,
                _ = authority_failed.cancelled() => break,
                outbound = out_rx.recv() => match outbound {
                    Some(outbound) => outbound,
                    None => break,
                },
            };
            queued_bytes.fetch_sub(outbound.line.len(), std::sync::atomic::Ordering::AcqRel);
            let actor = outbound_actor
                .read()
                .expect("IRC actor lock poisoned")
                .clone();
            if let Some(actor) = actor.as_ref()
                && auth.validate_actor(actor).await.is_err()
            {
                failed.cancel();
                break;
            }
            if let (Some(actor), Some(guard)) = (actor.as_ref(), outbound.guard.as_ref())
                && !engine.delivery_guard_is_current(actor, guard).await
            {
                if matches!(
                    guard,
                    crate::engine::user_session::DeliveryGuard::ServerPermissions(_)
                ) {
                    continue;
                }
                failed.cancel();
                break;
            }
            let sanitized = sanitize_outbound_line(&outbound.line);
            let data = format!("{sanitized}\r\n");
            let wrote = tokio::select! {
                _ = cancel.cancelled() => break,
                _ = failed.cancelled() => break,
                _ = authority_failed.cancelled() => break,
                result = tokio::time::timeout(
                    Duration::from_secs(5),
                    writer.write_all(data.as_bytes()),
                ) => matches!(result, Ok(Ok(()))),
            };
            if !wrote {
                failed.cancel();
                break;
            }
        }
    }
}
