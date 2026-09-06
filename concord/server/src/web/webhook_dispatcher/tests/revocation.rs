use super::*;

#[tokio::test]
async fn grant_revoked_while_waiting_for_egress_admission_sends_no_request() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (pool, vault, transport) = queued_delivery_fixture(address).await;
    let blocker_transport = transport.clone();
    let blocker_url = reqwest::Url::parse(&format!("http://{address}/block")).unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 2048];
        let _ = socket.read(&mut request).await.unwrap();
        accepted_tx.send(()).unwrap();
        release_rx.await.unwrap();
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(150), listener.accept()).await
    });
    let blocker = tokio::spawn(async move {
        let request = blocker_transport
            .request(
                reqwest::Method::GET,
                blocker_url,
                crate::egress::RedirectPolicy::Reject,
            )
            .unwrap();
        blocker_transport.send(request).await.unwrap();
    });
    accepted_rx.await.unwrap();
    let dispatcher = WebhookDispatcher::new(pool.clone(), transport, vault, 8);
    let worker_pool = pool.clone();
    let worker = tokio::spawn(async move {
        crate::jobs::run_once_matching(
            &worker_pool,
            "worker",
            &dispatcher,
            &crate::jobs::JobSelection {
                operation_types: &["webhook_delivery"],
                lease_seconds: 30,
                limit: 1,
                max_attempts: 8,
            },
        )
        .await
        .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    sqlx::query("UPDATE webhooks SET credential_state='revoked',revoked_at=datetime('now'),grant_version=grant_version+1 WHERE id='hook'")
        .execute(&pool).await.unwrap();
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    let report = worker.await.unwrap();
    assert_eq!(report.retried_or_failed, 1);
    assert!(
        server.await.unwrap().is_err(),
        "revoked delivery reached the network"
    );
}
