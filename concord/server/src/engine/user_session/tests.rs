use super::*;

fn event_with_payload(bytes: usize) -> ChatEvent {
    ChatEvent::Error {
        code: "test".into(),
        message: "x".repeat(bytes),
    }
}

#[tokio::test]
async fn queue_overflow_cancels_instead_of_dropping_silently() {
    let before = crate::runtime_metrics::snapshot();
    let overflow_index = crate::runtime_metrics::Operation::QueueOverflow as usize;
    let (tx, _rx) = mpsc::channel(MAX_OUTBOUND_QUEUE);
    let session = UserSession::new(
        ConnectionId::new(),
        Some("user".into()),
        "nick".into(),
        Protocol::WebSocket,
        tx,
        None,
    );
    for _ in 0..MAX_OUTBOUND_QUEUE {
        assert!(session.send(event_with_payload(1)));
    }
    assert!(!session.send(event_with_payload(1)));
    assert!(session.overflow_cancellation_token().is_cancelled());
    let after = crate::runtime_metrics::snapshot();
    assert!(after.failed[overflow_index] > before.failed[overflow_index]);
}

#[tokio::test]
async fn acknowledgment_and_resync_events_record_successful_queueing() {
    let before = crate::runtime_metrics::snapshot();
    let ack_index = crate::runtime_metrics::Operation::CommandAck as usize;
    let resync_index = crate::runtime_metrics::Operation::Resync as usize;
    let (tx, mut rx) = mpsc::channel(MAX_OUTBOUND_QUEUE);
    let session = UserSession::new(
        ConnectionId::new(),
        Some("user".into()),
        "nick".into(),
        Protocol::WebSocket,
        tx,
        None,
    );
    assert!(session.send(ChatEvent::MessageAck {
        id: crate::engine::ids::MessageId::from_stored("historical-ack").unwrap(),
        server_id: "server".into(),
        channel: "#general".into(),
        conversation_id: Some("conversation".into()),
        request_id: "request".into(),
        client_message_id: "client".into(),
        sequence: "1".into(),
        persisted_at: "2026-09-06T00:00:00Z".into(),
        replayed: false,
        nonce: None,
    }));
    assert!(session.send(ChatEvent::ResyncRequired {
        request_id: "sync".into(),
        reason: crate::engine::replay::ResyncReason::ProtocolChanged,
    }));
    rx.recv().await.unwrap();
    session.take_delivery_guard();
    rx.recv().await.unwrap();
    session.take_delivery_guard();
    let after = crate::runtime_metrics::snapshot();
    assert!(after.succeeded[ack_index] > before.succeeded[ack_index]);
    assert!(after.succeeded[resync_index] > before.succeeded[resync_index]);
}

#[tokio::test]
async fn queued_byte_limit_cancels_oversized_response() {
    let (tx, _rx) = mpsc::channel(MAX_OUTBOUND_QUEUE);
    let session = UserSession::new(
        ConnectionId::new(),
        Some("user".into()),
        "nick".into(),
        Protocol::WebSocket,
        tx,
        None,
    );
    assert!(!session.send(event_with_payload(MAX_OUTBOUND_BYTES)));
    assert!(session.overflow_cancellation_token().is_cancelled());
}

#[tokio::test]
async fn sensitive_event_without_recoverable_scope_is_rejected() {
    let (tx, _rx) = mpsc::channel(MAX_OUTBOUND_QUEUE);
    let session = UserSession::new(
        ConnectionId::new(),
        Some("user".into()),
        "nick".into(),
        Protocol::WebSocket,
        tx,
        None,
    );
    assert!(!session.send_guarded(
        ChatEvent::SearchResults {
            request_id: None,
            server_id: "server".into(),
            query: "secret".into(),
            results: Vec::new(),
            total_count: 0,
            offset: 0,
            next_continuation: None,
            restarted: false,
        },
        None,
    ));
    assert!(session.overflow_cancellation_token().is_cancelled());
}

#[tokio::test]
async fn privileged_response_gets_a_permission_guard_from_its_type() {
    let (tx, mut rx) = mpsc::channel(MAX_OUTBOUND_QUEUE);
    let session = UserSession::new(
        ConnectionId::new(),
        Some("user".into()),
        "nick".into(),
        Protocol::WebSocket,
        tx,
        None,
    );
    assert!(session.send(ChatEvent::BanList {
        server_id: "server".into(),
        bans: Vec::new(),
    }));
    rx.recv().await.unwrap();
    assert!(matches!(
        session.take_delivery_guard(),
        Some(DeliveryGuard::ServerPermissions(requirements))
            if requirements == vec![("server".into(), super::super::permissions::Permissions::BAN_MEMBERS)]
    ));
}

#[tokio::test]
async fn large_role_bootstrap_and_scoped_mutation_do_not_overflow_healthy_reader() {
    let (tx, mut rx) = mpsc::channel(MAX_OUTBOUND_QUEUE);
    let session = UserSession::new(
        ConnectionId::new(),
        Some("user-0".into()),
        "nick".into(),
        Protocol::WebSocket,
        tx,
        None,
    );
    let members = (0..300)
        .map(|index| super::super::events::MemberRoleInfo {
            user_id: format!("user-{index}"),
            role_ids: vec!["colored".into()],
        })
        .collect();
    let role = super::super::events::RoleInfo {
        id: "colored".into(),
        server_id: "server".into(),
        name: "Colored".into(),
        color: Some("#123456".into()),
        icon_url: None,
        position: 1,
        permissions: 0,
        is_default: false,
    };
    assert!(session.send(ChatEvent::RoleList {
        server_id: "server".into(),
        version: 1,
        roles: vec![role],
        member_roles: Some(members),
    }));
    assert!(session.send(ChatEvent::RoleList {
        server_id: "server".into(),
        version: 2,
        roles: vec![],
        member_roles: None,
    }));
    assert!(session.send(ChatEvent::MemberRoleUpdate {
        server_id: "server".into(),
        version: 2,
        user_id: "user-0".into(),
        role_ids: vec![],
    }));
    for _ in 0..3 {
        rx.recv().await.expect("queued role projection");
        assert!(session.take_delivery_guard().is_some());
    }
    assert!(!session.overflow_cancellation_token().is_cancelled());
}
