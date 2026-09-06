use super::*;

#[test]
fn irc_tls_loader_accepts_certificate_chain_and_private_key_pem() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    load_irc_tls_config(
        &fixtures.join("irc-tls-cert.pem"),
        &fixtures.join("irc-tls-key.pem"),
    )
    .unwrap();
}

#[tokio::test]
async fn drain_observes_clean_supervised_completion() {
    let mut tasks = JoinSet::new();
    tasks.spawn(async { ("test task", Ok(())) });

    drain_tasks(&mut tasks, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn drain_aborts_tasks_after_deadline() {
    let mut tasks = JoinSet::new();
    tasks.spawn(async {
        std::future::pending::<()>().await;
        ("stuck task", Ok(()))
    });

    let error = drain_tasks(&mut tasks, Duration::from_millis(1))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("deadline"));
    assert!(tasks.is_empty());
}
