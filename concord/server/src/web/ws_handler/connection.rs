use super::{
    Actor, AppState, Arc, AuthService, ChatEngine, ChatEvent, CookieJar, Extension, HeaderMap,
    Instant, IntoResponse, Message, Protocol, SinkExt, State, StreamExt, WebSocket,
    WebSocketUpgrade, error, fixed_window_admit, handle_client_message, info, users, warn,
    websocket_command_correlation, websocket_command_is_read,
};

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    Extension(rate_limiters): Extension<Arc<crate::web::rate_limit::ApiRateLimiters>>,
    jar: CookieJar,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let expected_origin = state.auth_config.public_url.trim_end_matches('/');
    if headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        != Some(expected_origin)
    {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Invalid WebSocket origin",
        )
            .into_response();
    }
    // Browser sessions use the same-site cookie. Bot clients use the canonical
    // bearer credential rather than trying to impersonate a browser session.
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if jar.get("concord_session").is_some() && bearer.is_some() {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Provide exactly one WebSocket credential",
        )
            .into_response();
    }
    let (nickname, actor, avatar_url) = if let Some(cookie) = jar.get("concord_session") {
        let actor = match state.auth.authenticate_web_session(cookie.value()).await {
            Ok(actor) => actor,
            Err(error) => {
                return crate::web::auth_middleware::auth_error_response(
                    error,
                    "Invalid session token",
                );
            }
        };
        match users::get_user(&state.db, actor.user_id().as_str()).await {
            Ok(Some((_id, username, _email, avatar))) => (username, actor, avatar),
            Ok(None) => {
                return (axum::http::StatusCode::UNAUTHORIZED, "User not found").into_response();
            }
            Err(_) => {
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Authentication service unavailable",
                )
                    .into_response();
            }
        }
    } else if let Some(token) = bearer {
        let actor = match state.auth.authenticate_bot(token).await {
            Ok(actor) => actor,
            Err(error) => {
                return crate::web::auth_middleware::auth_error_response(
                    error,
                    "Invalid bot token",
                );
            }
        };
        match users::get_user(&state.db, actor.user_id().as_str()).await {
            Ok(Some((_id, username, _email, avatar))) => (username, actor, avatar),
            Ok(None) => {
                return (axum::http::StatusCode::UNAUTHORIZED, "Bot user not found")
                    .into_response();
            }
            Err(_) => {
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Authentication service unavailable",
                )
                    .into_response();
            }
        }
    } else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Not authenticated. Provide a valid session cookie or bot bearer token.",
        )
            .into_response();
    };

    if !rate_limiters.admit_authenticated_ws(actor.credential_id().as_str()) {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Too many authenticated reconnects. Please try again later.",
        )
            .into_response();
    }

    let engine = state.engine.clone();
    let auth = state.auth.clone();
    let shutdown = state.shutdown.clone();
    ws.max_message_size(64 * 1024) // 64 KB max WS message
        .on_upgrade(move |socket| {
            handle_ws_connection(socket, engine, auth, actor, nickname, avatar_url, shutdown)
        })
        .into_response()
}

pub(super) async fn handle_ws_connection(
    socket: WebSocket,
    engine: Arc<ChatEngine>,
    auth: AuthService,
    actor: Actor,
    nickname: String,
    avatar_url: Option<String>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let credential_lease = match auth.register_live(&actor).await {
        Ok(lease) => lease,
        Err(_) => return,
    };
    let (session_id, mut event_rx) = match engine.connect(
        Some(actor.user_id().as_str().to_owned()),
        nickname.clone(),
        Protocol::WebSocket,
        avatar_url,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            warn!(%nickname, error = %e, "WebSocket connection rejected");
            return;
        }
    };
    if engine
        .bind_authenticated_actor(session_id, actor.clone())
        .is_err()
    {
        engine.disconnect(session_id);
        return;
    }
    if engine.send_own_presence(session_id).await.is_err() {
        engine.disconnect(session_id);
        return;
    }
    let Some(session) = engine.get_session(session_id) else {
        return;
    };
    let overflow_cancel = session.overflow_cancellation_token();

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (heartbeat_tx, mut heartbeat_rx) = tokio::sync::mpsc::channel::<()>(1);

    let writer_auth = auth.clone();
    let writer_actor = actor.clone();
    let writer_cancel = credential_lease.cancellation_token();
    let writer_shutdown = shutdown.clone();
    let writer_engine = engine.clone();
    let writer_session = session.clone();
    let writer_overflow = overflow_cancel.clone();
    let writer_done = tokio_util::sync::CancellationToken::new();
    let writer_finished = writer_done.clone();
    let write_handle = tokio::spawn(async move {
        enum WriterAction {
            Event(Box<ChatEvent>),
            Ping,
            PongReceived,
        }

        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        heartbeat.tick().await;
        let mut awaiting_pong = false;
        loop {
            let action = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                _ = writer_shutdown.cancelled() => break,
                _ = writer_overflow.cancelled() => break,
                _ = crate::auth::authority::wait_for_expiry(writer_actor.expires_at()) => break,
                _ = heartbeat.tick() => WriterAction::Ping,
                pong = heartbeat_rx.recv() => match pong {
                    Some(()) => WriterAction::PongReceived,
                    None => break,
                },
                event = event_rx.recv() => match event {
                    Some(event) => WriterAction::Event(Box::new(event)),
                    None => break,
                },
            };
            if matches!(action, WriterAction::PongReceived) {
                awaiting_pong = false;
                continue;
            }
            if matches!(action, WriterAction::Ping) && awaiting_pong {
                break;
            }
            if writer_auth.validate_actor(&writer_actor).await.is_err() {
                break;
            }
            let message = match action {
                WriterAction::Event(event) => {
                    if let Some(guard) = writer_session.take_delivery_guard() {
                        let authorized = writer_engine
                            .delivery_guard_is_current(&writer_actor, &guard)
                            .await;
                        if !authorized {
                            warn!(
                                guard = guard.kind(),
                                "WebSocket delivery guard denied queued event"
                            );
                            if matches!(
                                guard,
                                crate::engine::user_session::DeliveryGuard::ServerPermissions(_)
                            ) {
                                continue;
                            }
                            break;
                        }
                    }
                    match serde_json::to_string(&event) {
                        Ok(json) => Message::Text(json.into()),
                        Err(e) => {
                            error!(error = %e, "failed to serialize event");
                            continue;
                        }
                    }
                }
                WriterAction::Ping => {
                    awaiting_pong = true;
                    Message::Ping(Vec::new().into())
                }
                WriterAction::PongReceived => unreachable!(),
            };
            let sent = tokio::select! {
                _ = writer_cancel.cancelled() => false,
                _ = writer_shutdown.cancelled() => false,
                _ = writer_overflow.cancelled() => false,
                result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ws_sender.send(message),
                ) => matches!(result, Ok(Ok(()))),
            };
            if !sent {
                break;
            }
        }
        writer_finished.cancel();
    });

    let engine_ref = engine.clone();
    let mut ws_mutation_count: u32 = 0;
    let mut ws_mutation_window_start = Instant::now();
    let mut ws_read_count: u32 = 0;
    let mut ws_read_window_start = Instant::now();
    const WS_COMMANDS_PER_SECOND: u32 = 30;
    const WS_READS_PER_SECOND: u32 = 120;

    loop {
        let msg = match tokio::select! {
            _ = credential_lease.cancelled() => break,
            _ = shutdown.cancelled() => break,
            _ = crate::auth::authority::wait_for_expiry(actor.expires_at()) => break,
            _ = writer_done.cancelled() => break,
            _ = overflow_cancel.cancelled() => break,
            result = ws_receiver.next() => result,
        } {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => {
                warn!(error = %e, "WebSocket read error");
                break;
            }
            None => break, // Stream closed
        };

        match msg {
            Message::Text(text) => {
                if auth.validate_actor(&actor).await.is_err() {
                    break;
                }
                let is_read = websocket_command_is_read(&text);
                let admitted = if is_read {
                    fixed_window_admit(
                        &mut ws_read_count,
                        &mut ws_read_window_start,
                        WS_READS_PER_SECOND,
                    )
                } else {
                    fixed_window_admit(
                        &mut ws_mutation_count,
                        &mut ws_mutation_window_start,
                        WS_COMMANDS_PER_SECOND,
                    )
                };
                if !admitted {
                    if let Some(session) = engine_ref.get_session(session_id) {
                        let message = if is_read {
                            "Rate limited: too many read commands; retry shortly"
                        } else {
                            "Rate limited: too many mutation commands; retry shortly"
                        };
                        if let Some(request_id) = websocket_command_correlation(&text) {
                            let _ = session.send(ChatEvent::CommandError {
                                request_id,
                                code: "RATE_LIMITED".into(),
                                message: message.into(),
                                retryable: true,
                            });
                        } else {
                            let _ = session.send(ChatEvent::Error {
                                code: "RATE_LIMITED".into(),
                                message: message.into(),
                            });
                        }
                    }
                    continue;
                }
                handle_client_message(&engine_ref, session_id, &text).await;
            }
            Message::Pong(_) => {
                let _ = heartbeat_tx.try_send(());
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    engine.disconnect(session_id);
    write_handle.abort();
    info!(%session_id, %nickname, "WebSocket connection closed");
}
