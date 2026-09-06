use super::*;

#[tokio::test]
async fn actual_http_and_websocket_upgrade_reject_unregistered_credentials() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let legacy = create_session_token("user-1", "session-secret", 1).unwrap();
    let router = app(
        pool.clone(),
        AuthService::new(pool, "session-secret".into(), 1),
    )
    .await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", format!("concord_session={legacy}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "GET /ws HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nOrigin: http://localhost:3000\r\nCookie: concord_session={legacy}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .await
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response:?}");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_origin_and_cookie_mutation_are_rejected() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let router = app(pool, auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let (_, wrong_origin) =
        websocket_handshake(address, &token, Some("https://attacker.test")).await;
    assert!(wrong_origin.starts_with("HTTP/1.1 403"), "{wrong_origin:?}");
    let (_, missing_origin) = websocket_handshake(address, &token, None).await;
    assert!(
        missing_origin.starts_with("HTTP/1.1 403"),
        "{missing_origin:?}"
    );
    let (_, changed_cookie) = websocket_handshake(
        address,
        &format!("{token}mutated"),
        Some("http://localhost:3000"),
    )
    .await;
    assert!(
        changed_cookie.starts_with("HTTP/1.1 401"),
        "{changed_cookie:?}"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_websocket_logout_revocation_stops_commands_and_cleans_engine_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = auth.issue_web_session("user-1").await.unwrap();
    let (router, engine, _) = app_runtime(pool.clone(), auth.clone()).await;
    let logout_router = router.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, response) =
        websocket_handshake(address, &token, Some("http://localhost:3000")).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response:?}");
    wait_for_session(&engine, "carmilla", true).await;

    sqlx::query(
        "CREATE TRIGGER fail_logout_revocation BEFORE UPDATE OF revoked_at ON auth_credentials \
         BEGIN SELECT RAISE(FAIL, 'injected logout revocation failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let failed = logout_router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        failed
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .is_none(),
        "failed revocation cleared the browser cookie"
    );
    let revoked_at: Option<i64> =
        sqlx::query_scalar("SELECT revoked_at FROM auth_credentials WHERE id=?")
            .bind(actor.credential_id().as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        revoked_at.is_none(),
        "failed revocation changed durable state"
    );
    auth.authenticate_web_session(&token).await.unwrap();
    wait_for_session(&engine, "carmilla", true).await;

    sqlx::query("DROP TRIGGER fail_logout_revocation")
        .execute(&pool)
        .await
        .unwrap();
    let retried = logout_router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(retried.status().is_redirection());
    assert!(
        retried.headers()[axum::http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let revoked_at: Option<i64> =
        sqlx::query_scalar("SELECT revoked_at FROM auth_credentials WHERE id=?")
            .bind(actor.credential_id().as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked_at.is_some(), "successful retry was not durable");
    assert!(matches!(
        auth.authenticate_web_session(&token).await,
        Err(AuthError::Revoked)
    ));
    let already_revoked = logout_router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(already_revoked.status().is_redirection());
    assert!(
        already_revoked.headers()[axum::http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    wait_for_ws_close(&mut socket).await;
    wait_for_session(&engine, "carmilla", false).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_websocket_durable_expiry_rejects_next_command_and_cleans_engine_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = auth.issue_web_session("user-1").await.unwrap();
    let (router, engine, _) = app_runtime(pool.clone(), auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, response) =
        websocket_handshake(address, &token, Some("http://localhost:3000")).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response:?}");
    wait_for_session(&engine, "carmilla", true).await;

    sqlx::query(
        "UPDATE auth_credentials SET expires_at=unixepoch()-1, version=version+1 WHERE id=?",
    )
    .bind(actor.credential_id().as_str())
    .execute(&pool)
    .await
    .unwrap();
    send_ws_text(&mut socket, r#"{"type":"list_servers"}"#).await;
    wait_for_ws_close(&mut socket).await;
    wait_for_session(&engine, "carmilla", false).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_websocket_observes_shared_shutdown_and_cleans_engine_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, _) = auth.issue_web_session("user-1").await.unwrap();
    let (router, engine, shutdown) = app_runtime(pool, auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut socket, response) =
        websocket_handshake(address, &token, Some("http://localhost:3000")).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response:?}");
    wait_for_session(&engine, "carmilla", true).await;

    shutdown.cancel();
    wait_for_ws_close(&mut socket).await;
    wait_for_session(&engine, "carmilla", false).await;
    server.abort();
}

#[tokio::test]
async fn actual_http_logout_durably_revokes_and_notifies_live_session() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let (token, actor) = auth.issue_web_session("user-1").await.unwrap();
    let lease = auth.register_live(&actor).await.unwrap();
    let router = app(pool.clone(), auth).await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", format!("concord_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());
    tokio::time::timeout(Duration::from_secs(1), lease.cancelled())
        .await
        .unwrap();
    assert!(matches!(
        AuthService::new(pool, "session-secret".into(), 1)
            .authenticate_web_session(&token)
            .await,
        Err(AuthError::Revoked)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_irc_connection_closes_and_releases_engine_session_on_revocation() {
    let pool = database().await;
    user(&pool, "user-1", "carmilla").await;
    let auth = AuthService::new(pool.clone(), "session-secret".into(), 1);
    let web_actor = auth.issue_web_session("user-1").await.unwrap().1;
    let issued = auth
        .issue_irc_token(web_actor.user_id(), Some("test"))
        .await
        .unwrap();
    let credential_id = issued.credential_id.clone();
    let irc_actor = auth
        .authenticate_irc(&issued.secret, "carmilla")
        .await
        .unwrap();
    let canonical_nick = auth.canonical_irc_nickname(&irc_actor).await.unwrap();
    let engine = Arc::new(ChatEngine::new(
        pool.clone(),
        auth.clone(),
        "session-secret",
        4000,
        100,
    ));
    let (server, client) = tokio::io::duplex(4096);
    let hold_writes = Arc::new(AtomicBool::new(false));
    let write_entered = CancellationToken::new();
    let server = HeldWriteStream {
        inner: server,
        hold: hold_writes.clone(),
        entered: write_entered.clone(),
    };
    let cancel = CancellationToken::new();
    let task = tokio::spawn(handle_irc_connection_until(
        server,
        "test-peer".into(),
        engine.clone(),
        pool,
        auth.clone(),
        cancel,
    ));
    let (reader, mut writer) = tokio::io::split(client);
    let mut reader = BufReader::new(reader);
    writer
        .write_all(
            format!(
                "PASS {}\r\nNICK carmilla\r\nUSER carmilla 0 * :Carmilla\r\n",
                issued.secret
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut line = String::new();
    loop {
        tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        if line.contains(&format!(" 001 {canonical_nick} ")) {
            break;
        }
        line.clear();
    }
    assert!(engine.get_session_id_by_nick(&canonical_nick).is_some());

    hold_writes.store(true, Ordering::Release);
    writer.write_all(b"CAP LS\r\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), write_entered.cancelled())
        .await
        .expect("IRC writer did not enter the held sink");
    auth.revoke_credential(&credential_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("revoked IRC connection remained live")
        .unwrap();
    assert!(engine.get_session_id_by_nick(&canonical_nick).is_none());
}
