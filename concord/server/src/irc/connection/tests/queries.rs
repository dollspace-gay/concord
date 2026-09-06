use super::*;

#[test]
fn test_channel_list_is_silent() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::ChannelList {
            server_id: DEFAULT_SERVER_ID.into(),
            channels: vec![],
        },
    );
    assert!(lines.is_empty());
}

#[test]
fn outbound_byte_budget_marks_transport_failed_before_enqueue() {
    let (tx, mut rx) = mpsc::channel::<OutboundLine>(MAX_OUTBOUND_DESCRIPTORS);
    let failed = CancellationToken::new();
    let out = Outbound {
        tx,
        failed: failed.clone(),
        actor: Arc::new(std::sync::RwLock::new(None)),
        queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    send_line(&out, &"x".repeat(MAX_OUTBOUND_BYTES + 1));
    assert!(failed.is_cancelled());
    assert!(rx.try_recv().is_err());
}
