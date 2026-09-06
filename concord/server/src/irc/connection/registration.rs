use super::{
    AuthService, CancellationToken, ChatEngine, ChatEvent, ClientCaps, IrcMessage, MOTD_LINES,
    Outbound, Protocol, RegState, SUPPORTED_CAPS, formatter, mpsc, send_line, warn,
};

pub(super) struct Registration<'a> {
    pub engine: &'a ChatEngine,
    pub auth: &'a AuthService,
    pub out: &'a Outbound,
    pub authority_failed: &'a CancellationToken,
    pub state: &'a mut RegState,
    pub caps: &'a mut ClientCaps,
    pub event_rx: &'a mut Option<mpsc::Receiver<ChatEvent>>,
    pub credential_lease: &'a mut Option<crate::auth::authority::CredentialLease>,
    pub credential_expiry: &'a mut Option<i64>,
}

pub(super) async fn register(
    context: Registration<'_>,
    msg: IrcMessage,
) -> std::ops::ControlFlow<()> {
    let Registration {
        engine,
        auth,
        out,
        authority_failed,
        state,
        caps,
        event_rx,
        credential_lease,
        credential_expiry,
    } = context;
    // Handle CAP during registration
    if msg.command == "CAP" {
        let sn = formatter::server_name();
        match msg.params.first().map(|s| s.as_str()) {
            Some("LS") => {
                send_line(out, &format!(":{sn} CAP * LS :{SUPPORTED_CAPS}"));
            }
            Some("REQ") => {
                // Client requests specific capabilities
                if let Some(requested) = msg.params.get(1) {
                    let mut ack = Vec::new();
                    for cap in requested.split_whitespace() {
                        match cap {
                            "server-time" => {
                                caps.server_time = true;
                                ack.push(cap);
                            }
                            "message-tags" => {
                                caps.message_tags = true;
                                ack.push(cap);
                            }
                            "sasl" => {
                                caps.sasl = true;
                                ack.push(cap);
                            }
                            _ => {} // Ignore unsupported caps
                        }
                    }
                    if !ack.is_empty() {
                        send_line(out, &format!(":{sn} CAP * ACK :{}", ack.join(" ")));
                    }
                }
            }
            Some("END") => {} // Falls through to registration check
            _ => {}
        }
        return std::ops::ControlFlow::Continue(());
    }

    // Handle SASL AUTHENTICATE during registration
    if msg.command == "AUTHENTICATE" {
        let sn = formatter::server_name();
        if let Some(param) = msg.params.first() {
            if param == "PLAIN" {
                // Acknowledge, ask for credentials
                send_line(out, "AUTHENTICATE +");
            } else if param == "*" {
                // Client aborts SASL
                send_line(out, &format!(":{sn} 906 * :SASL authentication aborted"));
            } else {
                // base64 payload: \0username\0token
                use base64::Engine as _;
                let decoded = base64::engine::general_purpose::STANDARD.decode(param);
                if let Ok(bytes) = decoded {
                    // Split on NUL: [authzid, authcid, passwd]
                    let parts: Vec<&[u8]> = bytes.splitn(3, |&b| b == 0).collect();
                    if parts.len() == 3 {
                        let _authzid = String::from_utf8_lossy(parts[0]);
                        let authcid = String::from_utf8_lossy(parts[1]);
                        let passwd = String::from_utf8_lossy(parts[2]);
                        // Validate the token
                        let nick_hint = if authcid.is_empty() { "*" } else { &authcid };
                        match auth.authenticate_irc(&passwd, nick_hint).await {
                            Ok(actor) => {
                                if let RegState::Unregistered { ref mut pass, .. } = *state {
                                    *pass = Some(passwd.into_owned());
                                }
                                send_line(
                                    out,
                                    &format!(
                                        ":{sn} 900 * {} :You are now logged in as {}",
                                        actor.user_id().as_str(),
                                        actor.user_id().as_str(),
                                    ),
                                );
                                send_line(
                                    out,
                                    &format!(":{sn} 903 * :SASL authentication successful"),
                                );
                            }
                            Err(_) => {
                                send_line(out, &format!(":{sn} 904 * :SASL authentication failed"));
                            }
                        }
                    } else {
                        send_line(out, &format!(":{sn} 904 * :SASL authentication failed"));
                    }
                } else {
                    send_line(out, &format!(":{sn} 904 * :SASL authentication failed"));
                }
            }
        }
        return std::ops::ControlFlow::Continue(());
    }

    // Process registration commands
    match msg.command.as_str() {
        "PASS" => {
            if let RegState::Unregistered { ref mut pass, .. } = *state {
                *pass = msg.params.first().cloned();
            }
        }
        "NICK" => {
            let Some(wanted_nick) = msg.params.first() else {
                send_line(out, &formatter::err_nonicknamegiven("*"));
                return std::ops::ControlFlow::Continue(());
            };

            if !engine.is_nick_available(wanted_nick) {
                send_line(out, &formatter::err_nicknameinuse("*", wanted_nick));
                return std::ops::ControlFlow::Continue(());
            }

            if let RegState::Unregistered { ref mut nick, .. } = *state {
                *nick = Some(wanted_nick.clone());
            }
        }
        "USER" => {
            if let RegState::Unregistered {
                ref mut user_received,
                ..
            } = *state
            {
                *user_received = true;
            }
        }
        "QUIT" => return std::ops::ControlFlow::Break(()),
        _ => {
            send_line(out, &formatter::err_notregistered());
            return std::ops::ControlFlow::Continue(());
        }
    }

    // Check if registration is complete
    if let RegState::Unregistered {
        ref pass,
        ref nick,
        user_received,
    } = *state
        && let (Some(nick_val), true) = (nick.as_ref(), user_received)
    {
        // If a PASS was provided, validate it as an IRC token
        let user_id = if let Some(pass_token) = pass {
            match auth.authenticate_irc(pass_token, nick_val).await {
                Ok(actor) => Some(actor),
                Err(crate::auth::authority::AuthError::Invalid) => {
                    send_line(
                        out,
                        &format!(
                            ":{} 464 {} :Password incorrect",
                            formatter::server_name(),
                            nick_val,
                        ),
                    );
                    return std::ops::ControlFlow::Break(());
                }
                Err(e) => {
                    warn!(error = %e, "IRC token validation error");
                    send_line(
                        out,
                        &format!(
                            ":{} 464 {} :Authentication error",
                            formatter::server_name(),
                            nick_val,
                        ),
                    );
                    return std::ops::ControlFlow::Break(());
                }
            }
        } else {
            // No PASS provided — reject anonymous connections
            send_line(
                out,
                &format!(
                    ":{} 464 {} :You must provide a password (PASS) to connect. Generate an IRC token in the web UI.",
                    formatter::server_name(),
                    nick_val,
                ),
            );
            return std::ops::ControlFlow::Break(());
        };

        // Try to register with the engine
        let actor = user_id.expect("authenticated registration has actor");
        let canonical_nick = match auth.canonical_irc_nickname(&actor).await {
            Ok(nickname) => nickname,
            Err(error) => {
                warn!(%error, "IRC canonical nickname lookup failed");
                send_line(
                    out,
                    &format!(
                        ":{} 464 {} :Authentication error",
                        formatter::server_name(),
                        nick_val,
                    ),
                );
                return std::ops::ControlFlow::Break(());
            }
        };
        match engine.connect(
            Some(actor.user_id().as_str().to_owned()),
            canonical_nick.clone(),
            Protocol::Irc,
            None,
        ) {
            Ok((sid, rx)) => {
                let nick_owned = canonical_nick;
                if engine.bind_authenticated_actor(sid, actor.clone()).is_err() {
                    engine.disconnect(sid);
                    return std::ops::ControlFlow::Break(());
                }
                *credential_lease = auth.register_live(&actor).await.ok();
                *credential_expiry = actor.expires_at();
                if credential_lease.is_none() {
                    engine.disconnect(sid);
                    return std::ops::ControlFlow::Break(());
                }
                let credential_cancel = credential_lease
                    .as_ref()
                    .expect("credential lease was checked")
                    .cancellation_token();
                let authority_cancel = authority_failed.clone();
                let expires_at = actor.expires_at();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = credential_cancel.cancelled() => {},
                        _ = crate::auth::authority::wait_for_expiry(expires_at) => {},
                    }
                    authority_cancel.cancel();
                });
                *out.actor.write().expect("IRC actor lock poisoned") = Some(actor.clone());

                // Send welcome burst
                send_line(out, &formatter::rpl_welcome(&nick_owned));
                send_line(out, &formatter::rpl_yourhost(&nick_owned));
                send_line(out, &formatter::rpl_created(&nick_owned));
                send_line(out, &formatter::rpl_myinfo(&nick_owned));

                // Send MOTD or ERR_NOMOTD
                let motd = MOTD_LINES.get();
                if let Some(lines) = motd
                    && !lines.is_empty()
                {
                    send_line(out, &formatter::rpl_motdstart(&nick_owned));
                    for line in lines {
                        send_line(out, &formatter::rpl_motd(&nick_owned, line));
                    }
                    send_line(out, &formatter::rpl_endofmotd(&nick_owned));
                } else {
                    send_line(out, &formatter::err_nomotd(&nick_owned));
                }

                *state = RegState::Registered {
                    session_id: sid,
                    nick: nick_owned,
                    actor: actor.clone(),
                };
                *event_rx = Some(rx);
            }
            Err(e) => {
                warn!(error = %e, "IRC registration failed");
                send_line(out, &formatter::err_nicknameinuse("*", &canonical_nick));
            }
        }
    }
    std::ops::ControlFlow::Continue(())
}
