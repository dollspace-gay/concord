use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Registration must complete promptly so unauthenticated clients cannot retain sockets.
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(60);
/// Registered clients must answer periodic server heartbeat probes.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
/// Command rate limit: burst capacity (commands allowed in a rapid burst).
const CMD_RATE_BURST: f64 = 10.0;
/// Command rate limit: refill rate (commands per second).
const CMD_RATE_PER_SEC: f64 = 2.0;

/// Global MOTD lines, initialized at startup from config.
static MOTD_LINES: OnceLock<Vec<String>> = OnceLock::new();

/// Supported IRCv3 capabilities.
const SUPPORTED_CAPS: &str = "server-time message-tags sasl";

/// Tracks which IRCv3 capabilities a client has negotiated.
#[derive(Default)]
struct ClientCaps {
    server_time: bool,
    message_tags: bool,
    sasl: bool,
}

struct Outbound {
    tx: mpsc::Sender<OutboundLine>,
    failed: CancellationToken,
    actor: Arc<std::sync::RwLock<Option<Actor>>>,
    queued_bytes: Arc<std::sync::atomic::AtomicUsize>,
}

struct OutboundLine {
    line: String,
    guard: Option<crate::engine::user_session::DeliveryGuard>,
}

type GuardedCommandReplies = Result<(Vec<String>, crate::engine::user_session::DeliveryGuard), ()>;

const MAX_OUTBOUND_DESCRIPTORS: usize = 256;
const MAX_OUTBOUND_BYTES: usize = 1024 * 1024;

/// Set the MOTD lines from config. Call once at startup.
pub fn set_motd(lines: Vec<String>) {
    let _ = MOTD_LINES.set(lines);
}

/// Per-connection token bucket for command rate limiting.
struct CommandRateLimit {
    tokens: f64,
    last_refill: Instant,
}

impl CommandRateLimit {
    fn new() -> Self {
        Self {
            tokens: CMD_RATE_BURST,
            last_refill: Instant::now(),
        }
    }

    /// Returns true if the command is allowed, false if rate limited.
    fn check(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * CMD_RATE_PER_SEC).min(CMD_RATE_BURST);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

use crate::auth::authority::{Actor, AuthService};
use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::{ChatEvent, ConnectionId};
use crate::engine::user_session::Protocol;

use super::commands::{self, to_irc_channel};
use super::formatter;
use super::framing::IrcLineDecoder;
use super::parser::IrcMessage;
use crate::engine::permissions::Permissions;

/// IRC registration state machine.
/// Clients must send NICK and USER (optionally PASS first) before they are registered.
enum RegState {
    /// Waiting for NICK and USER.
    Unregistered {
        pass: Option<String>,
        nick: Option<String>,
        user_received: bool,
    },
    /// Fully registered with the chat engine.
    Registered {
        session_id: ConnectionId,
        nick: String,
        actor: Actor,
    },
}

/// Handle a single IRC client connection from accept to close.
/// Accepts any stream implementing AsyncRead + AsyncWrite (plain TCP or TLS).
pub async fn handle_irc_connection<S>(
    stream: S,
    peer: String,
    engine: Arc<ChatEngine>,
    _db: SqlitePool,
    auth: AuthService,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    handle_irc_connection_until(stream, peer, engine, _db, auth, CancellationToken::new()).await;
}

/// Handles one connection until the peer closes or the owning listener cancels it.
pub async fn handle_irc_connection_until<S>(
    stream: S,
    peer: String,
    engine: Arc<ChatEngine>,
    _db: SqlitePool,
    auth: AuthService,
    cancel: CancellationToken,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    handle_irc_connection_with_timing(stream, peer, engine, _db, auth, cancel, HEARTBEAT_INTERVAL)
        .await;
}

async fn handle_irc_connection_with_timing<S>(
    stream: S,
    peer: String,
    engine: Arc<ChatEngine>,
    _db: SqlitePool,
    auth: AuthService,
    cancel: CancellationToken,
    heartbeat_interval: Duration,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    info!(%peer, "IRC client connected");

    let (reader, writer) = tokio::io::split(stream);
    let mut reader = reader;
    let mut writer = writer;

    // Bounded channel for outbound lines. Saturation marks the transport failed so
    // a slow client cannot silently lose events while retaining an engine session.
    let (out_tx, mut out_rx) = mpsc::channel::<OutboundLine>(MAX_OUTBOUND_DESCRIPTORS);
    let transport_failed = CancellationToken::new();
    let authority_failed = CancellationToken::new();
    let outbound_actor = Arc::new(std::sync::RwLock::new(None));
    let queued_bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let out = Outbound {
        tx: out_tx,
        failed: transport_failed.clone(),
        actor: outbound_actor.clone(),
        queued_bytes: queued_bytes.clone(),
    };

    // Spawn writer task
    let writer_failed = transport_failed.clone();
    let writer_auth = auth.clone();
    let writer_engine = engine.clone();
    let writer_cancel = cancel.clone();
    let writer_authority_failed = authority_failed.clone();
    let write_handle = tokio::spawn(async move {
        loop {
            let outbound = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                _ = writer_failed.cancelled() => break,
                _ = writer_authority_failed.cancelled() => break,
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
                && writer_auth.validate_actor(actor).await.is_err()
            {
                writer_failed.cancel();
                break;
            }
            if let (Some(actor), Some(guard)) = (actor.as_ref(), outbound.guard.as_ref())
                && !writer_engine.delivery_guard_is_current(actor, guard).await
            {
                if matches!(
                    guard,
                    crate::engine::user_session::DeliveryGuard::ServerPermissions(_)
                ) {
                    continue;
                }
                writer_failed.cancel();
                break;
            }
            let sanitized = sanitize_outbound_line(&outbound.line);
            let data = format!("{sanitized}\r\n");
            let wrote = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                _ = writer_failed.cancelled() => break,
                _ = writer_authority_failed.cancelled() => break,
                result = tokio::time::timeout(
                    Duration::from_secs(5),
                    writer.write_all(data.as_bytes()),
                ) => matches!(result, Ok(Ok(()))),
            };
            if !wrote {
                writer_failed.cancel();
                break;
            }
        }
    });

    let mut state = RegState::Unregistered {
        pass: None,
        nick: None,
        user_received: false,
    };

    let mut decoder = IrcLineDecoder::new();
    let mut event_rx: Option<mpsc::Receiver<ChatEvent>> = None;
    let mut credential_lease: Option<crate::auth::authority::CredentialLease> = None;
    let mut credential_expiry: Option<i64> = None;
    let mut cmd_rate = CommandRateLimit::new();
    let mut caps = ClientCaps::default();
    let registration_deadline = tokio::time::Instant::now() + REGISTRATION_TIMEOUT;
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut awaiting_pong: Option<String> = None;

    loop {
        // When registered, also select on engine events
        if let Some(ref mut rx) = event_rx {
            let registered_session = match state {
                RegState::Registered { session_id, .. } => engine.get_session(session_id),
                RegState::Unregistered { .. } => None,
            };
            let overflow_cancel = registered_session
                .as_ref()
                .map(|session| session.overflow_cancellation_token())
                .expect("registered IRC state has an engine session");
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = transport_failed.cancelled() => break,
                _ = overflow_cancel.cancelled() => break,
                _ = credential_lease.as_ref().expect("registered credentials have cancellation").cancelled() => break,
                _ = crate::auth::authority::wait_for_expiry(credential_expiry) => break,
                _ = heartbeat.tick() => {
                    if awaiting_pong.is_some() {
                        warn!(%peer, "IRC heartbeat timed out");
                        break;
                    }
                    let nonce = format!("{}-{}", formatter::server_name(), chrono::Utc::now().timestamp_millis());
                    send_line(&out, &format!("PING :{nonce}"));
                    awaiting_pong = Some(nonce);
                    continue;
                }
                result = decoder.read_line(&mut reader) => {
                    let line = match result {
                        Ok(Some(line)) => line,
                        Ok(None) | Err(_) => break,
                    };
                    if line.is_empty() {
                        continue;
                    }
                    let msg = match IrcMessage::parse(&line) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if msg.command == "PONG"
                        && msg.params.last() == awaiting_pong.as_ref()
                    {
                        awaiting_pong = None;
                        continue;
                    }

                    // Enforce per-connection command rate limit
                    let admitted = cmd_rate.check();
                    crate::runtime_metrics::record(
                        crate::runtime_metrics::Operation::CommandAdmission,
                        admitted,
                        Duration::ZERO,
                    );
                    if !admitted {
                        warn!(%peer, "IRC command rate limited");
                        continue;
                    }

                    if let RegState::Registered { ref session_id, ref nick, ref actor } = state {
                        if auth.validate_actor(actor).await.is_err() {
                            break;
                        }
                        if msg.command == "QUIT" {
                            let reason = msg.params.first().cloned();
                            send_line(&out, &format!(
                                "ERROR :Closing Link: {} (Quit: {})",
                                nick,
                                reason.as_deref().unwrap_or("Client quit")
                            ));
                            break;
                        }

                        // MOTD command — re-send MOTD on demand
                        if msg.command == "MOTD" {
                            let motd = MOTD_LINES.get();
                            if let Some(lines) = motd
                                && !lines.is_empty()
                            {
                                send_line(&out, &formatter::rpl_motdstart(nick));
                                for line in lines {
                                    send_line(&out, &formatter::rpl_motd(nick, line));
                                }
                                send_line(&out, &formatter::rpl_endofmotd(nick));
                            } else {
                                send_line(&out, &formatter::err_nomotd(nick));
                            }
                            continue;
                        }

                        // Async commands — need DB lookups or engine async methods
                        if matches!(msg.command.as_str(), "KICK" | "AWAY" | "INVITE" | "WHOIS" | "NAMES" | "WHO" | "LIST" | "HISTORY") {
                            if matches!(msg.command.as_str(), "NAMES" | "WHO" | "WHOIS" | "LIST" | "HISTORY") {
                                let guarded = match msg.command.as_str() {
                                    "NAMES" => handle_names_async(&engine, *session_id, nick, &msg).await,
                                    "WHO" => handle_who_async(&engine, *session_id, nick, &msg).await,
                                    "WHOIS" => handle_whois(&engine, *session_id, nick, &msg).await,
                                    "LIST" => handle_list_async(&engine, *session_id, nick, &msg).await,
                                    "HISTORY" => handle_history_async(&engine, *session_id, nick, &msg, &caps).await,
                                    _ => unreachable!(),
                                };
                                let Ok((replies, guard)) = guarded else {
                                    transport_failed.cancel(); break;
                                };
                                for reply in replies {
                                    send_line_guarded(&out, &reply, Some(guard.clone()));
                                }
                                continue;
                            }
                            let replies = match msg.command.as_str() {
                                "KICK" => handle_kick(&engine, *session_id, nick, &msg).await,
                                "AWAY" => handle_away(&engine, *session_id, nick, &msg).await,
                                "INVITE" => handle_invite(&engine, *session_id, nick, &msg).await,
                                "NAMES" | "WHO" | "WHOIS" | "LIST" | "HISTORY" => unreachable!(),
                                _ => unreachable!(),
                            };
                            let Ok(guard) = irc_command_delivery_guard(&engine, actor, &msg).await else {
                                transport_failed.cancel(); break;
                            };
                            for reply in replies {
                                send_line_guarded(&out, &reply, Some(guard.clone()));
                            }
                            continue;
                        }

                        let replies = commands::handle_command(&engine, *session_id, nick, &msg).await;
                        let Ok(guard) = irc_command_delivery_guard(&engine, actor, &msg).await else {
                            transport_failed.cancel();
                            break;
                        };
                        for reply in replies {
                            send_line_guarded(&out, &reply, Some(guard.clone()));
                        }
                    }
                }
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    if let RegState::Registered { ref nick, ref actor, .. } = state {
                        if auth.validate_actor(actor).await.is_err() {
                            break;
                        }
                        let delivery_guard = registered_session
                            .as_ref()
                            .and_then(|session| session.take_delivery_guard());
                        if let Some(guard) = delivery_guard.as_ref()
                            && !engine.delivery_guard_is_current(actor, guard).await
                        {
                            if matches!(
                                guard,
                                crate::engine::user_session::DeliveryGuard::ServerPermissions(_)
                            ) {
                                continue;
                            }
                            break;
                        }
                        let lines = event_to_irc_lines(&engine, nick, &event, &caps);
                        for line in lines {
                            send_line_guarded(&out, &line, delivery_guard.clone());
                        }
                    }
                }
            }
        } else {
            // Not registered yet — just read lines (with timeout)
            let line = tokio::select! {
                _ = cancel.cancelled() => break,
                _ = transport_failed.cancelled() => break,
                result = tokio::time::timeout_at(registration_deadline, decoder.read_line(&mut reader)) => {
                    match result {
                        Ok(Ok(Some(line))) => line,
                        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => break,
                    }
                }
            };

            if line.is_empty() {
                continue;
            }

            // Rate limit registration commands too (prevents brute-force token guessing)
            let admitted = cmd_rate.check();
            crate::runtime_metrics::record(
                crate::runtime_metrics::Operation::CommandAdmission,
                admitted,
                Duration::ZERO,
            );
            if !admitted {
                warn!(%peer, "IRC registration rate limited");
                continue;
            }

            let msg = match IrcMessage::parse(&line) {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Handle CAP during registration
            if msg.command == "CAP" {
                let sn = formatter::server_name();
                match msg.params.first().map(|s| s.as_str()) {
                    Some("LS") => {
                        send_line(&out, &format!(":{sn} CAP * LS :{SUPPORTED_CAPS}"));
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
                                send_line(&out, &format!(":{sn} CAP * ACK :{}", ack.join(" ")));
                            }
                        }
                    }
                    Some("END") => {} // Falls through to registration check
                    _ => {}
                }
                continue;
            }

            // Handle SASL AUTHENTICATE during registration
            if msg.command == "AUTHENTICATE" {
                let sn = formatter::server_name();
                if let Some(param) = msg.params.first() {
                    if param == "PLAIN" {
                        // Acknowledge, ask for credentials
                        send_line(&out, "AUTHENTICATE +");
                    } else if param == "*" {
                        // Client aborts SASL
                        send_line(&out, &format!(":{sn} 906 * :SASL authentication aborted"));
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
                                        if let RegState::Unregistered { ref mut pass, .. } = state {
                                            *pass = Some(passwd.into_owned());
                                        }
                                        send_line(
                                            &out,
                                            &format!(
                                                ":{sn} 900 * {} :You are now logged in as {}",
                                                actor.user_id().as_str(),
                                                actor.user_id().as_str(),
                                            ),
                                        );
                                        send_line(
                                            &out,
                                            &format!(":{sn} 903 * :SASL authentication successful"),
                                        );
                                    }
                                    Err(_) => {
                                        send_line(
                                            &out,
                                            &format!(":{sn} 904 * :SASL authentication failed"),
                                        );
                                    }
                                }
                            } else {
                                send_line(
                                    &out,
                                    &format!(":{sn} 904 * :SASL authentication failed"),
                                );
                            }
                        } else {
                            send_line(&out, &format!(":{sn} 904 * :SASL authentication failed"));
                        }
                    }
                }
                continue;
            }

            // Process registration commands
            match msg.command.as_str() {
                "PASS" => {
                    if let RegState::Unregistered { ref mut pass, .. } = state {
                        *pass = msg.params.first().cloned();
                    }
                }
                "NICK" => {
                    let Some(wanted_nick) = msg.params.first() else {
                        send_line(&out, &formatter::err_nonicknamegiven("*"));
                        continue;
                    };

                    if !engine.is_nick_available(wanted_nick) {
                        send_line(&out, &formatter::err_nicknameinuse("*", wanted_nick));
                        continue;
                    }

                    if let RegState::Unregistered { ref mut nick, .. } = state {
                        *nick = Some(wanted_nick.clone());
                    }
                }
                "USER" => {
                    if let RegState::Unregistered {
                        ref mut user_received,
                        ..
                    } = state
                    {
                        *user_received = true;
                    }
                }
                "QUIT" => break,
                _ => {
                    send_line(&out, &formatter::err_notregistered());
                    continue;
                }
            }

            // Check if registration is complete
            if let RegState::Unregistered {
                ref pass,
                ref nick,
                user_received,
            } = state
                && let (Some(nick_val), true) = (nick.as_ref(), user_received)
            {
                // If a PASS was provided, validate it as an IRC token
                let user_id = if let Some(pass_token) = pass {
                    match auth.authenticate_irc(pass_token, nick_val).await {
                        Ok(actor) => Some(actor),
                        Err(crate::auth::authority::AuthError::Invalid) => {
                            send_line(
                                &out,
                                &format!(
                                    ":{} 464 {} :Password incorrect",
                                    formatter::server_name(),
                                    nick_val,
                                ),
                            );
                            break;
                        }
                        Err(e) => {
                            warn!(error = %e, "IRC token validation error");
                            send_line(
                                &out,
                                &format!(
                                    ":{} 464 {} :Authentication error",
                                    formatter::server_name(),
                                    nick_val,
                                ),
                            );
                            break;
                        }
                    }
                } else {
                    // No PASS provided — reject anonymous connections
                    send_line(
                        &out,
                        &format!(
                            ":{} 464 {} :You must provide a password (PASS) to connect. Generate an IRC token in the web UI.",
                            formatter::server_name(),
                            nick_val,
                        ),
                    );
                    break;
                };

                // Try to register with the engine
                let actor = user_id.expect("authenticated registration has actor");
                let canonical_nick = match auth.canonical_irc_nickname(&actor).await {
                    Ok(nickname) => nickname,
                    Err(error) => {
                        warn!(%error, "IRC canonical nickname lookup failed");
                        send_line(
                            &out,
                            &format!(
                                ":{} 464 {} :Authentication error",
                                formatter::server_name(),
                                nick_val,
                            ),
                        );
                        break;
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
                            break;
                        }
                        credential_lease = auth.register_live(&actor).await.ok();
                        credential_expiry = actor.expires_at();
                        if credential_lease.is_none() {
                            engine.disconnect(sid);
                            break;
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
                        send_line(&out, &formatter::rpl_welcome(&nick_owned));
                        send_line(&out, &formatter::rpl_yourhost(&nick_owned));
                        send_line(&out, &formatter::rpl_created(&nick_owned));
                        send_line(&out, &formatter::rpl_myinfo(&nick_owned));

                        // Send MOTD or ERR_NOMOTD
                        let motd = MOTD_LINES.get();
                        if let Some(lines) = motd
                            && !lines.is_empty()
                        {
                            send_line(&out, &formatter::rpl_motdstart(&nick_owned));
                            for line in lines {
                                send_line(&out, &formatter::rpl_motd(&nick_owned, line));
                            }
                            send_line(&out, &formatter::rpl_endofmotd(&nick_owned));
                        } else {
                            send_line(&out, &formatter::err_nomotd(&nick_owned));
                        }

                        state = RegState::Registered {
                            session_id: sid,
                            nick: nick_owned,
                            actor: actor.clone(),
                        };
                        event_rx = Some(rx);
                    }
                    Err(e) => {
                        warn!(error = %e, "IRC registration failed");
                        send_line(&out, &formatter::err_nicknameinuse("*", &canonical_nick));
                    }
                }
            }
        }
    }

    // Disconnect from engine if registered
    if let RegState::Registered {
        session_id, nick, ..
    } = state
    {
        engine.disconnect(session_id);
        info!(%peer, %nick, "IRC client disconnected");
    } else {
        info!(%peer, "IRC client disconnected (unregistered)");
    }

    drop(event_rx);
    drop(out);
    let mut write_handle = write_handle;
    if tokio::time::timeout(Duration::from_secs(1), &mut write_handle)
        .await
        .is_err()
    {
        write_handle.abort();
        let _ = write_handle.await;
    }
}

/// Handle IRC KICK command: KICK #channel user [:reason]
/// Requires async because it does a DB lookup (nickname → user_id) and calls engine.kick_member().
async fn resolve_registered_channel(
    engine: &ChatEngine,
    session_id: ConnectionId,
    irc_name: &str,
) -> Result<(String, String), String> {
    let actor = engine
        .get_authenticated_actor(session_id)
        .ok_or_else(|| "authentication unavailable".to_string())?;
    engine.resolve_irc_channel_for_actor(&actor, irc_name).await
}

async fn irc_command_delivery_guard(
    engine: &ChatEngine,
    actor: &Actor,
    message: &IrcMessage,
) -> Result<crate::engine::user_session::DeliveryGuard, ()> {
    use crate::engine::user_session::DeliveryGuard;

    if message.command == "LIST" {
        let alias = message.params.first().and_then(|pattern| {
            pattern
                .strip_prefix('#')
                .unwrap_or(pattern)
                .strip_suffix("/*")
        });
        return engine
            .resolve_irc_server_for_actor(actor, alias)
            .await
            .map(|server_id| DeliveryGuard::ServerMembership(vec![server_id]))
            .map_err(|_| ());
    }

    let channel_parameter = match message.command.as_str() {
        "INVITE" => message.params.get(1),
        "JOIN" | "PART" | "PRIVMSG" | "TOPIC" | "MODE" | "NAMES" | "WHO" | "HISTORY" | "KICK" => {
            message.params.first()
        }
        _ => None,
    };
    let Some(channel_parameter) = channel_parameter else {
        return Ok(DeliveryGuard::ActorCurrent);
    };
    let mut channel_ids = Vec::new();
    for irc_name in channel_parameter
        .split(',')
        .filter(|name| name.starts_with('#'))
    {
        let Ok((server_id, channel_name)) =
            engine.resolve_irc_channel_for_actor(actor, irc_name).await
        else {
            return Err(());
        };
        let Ok(channel_id) = engine.resolve_channel_id(&server_id, &channel_name) else {
            return Err(());
        };
        channel_ids.push(channel_id);
    }
    if channel_ids.is_empty() {
        Ok(DeliveryGuard::ActorCurrent)
    } else if message.command == "HISTORY" {
        Ok(DeliveryGuard::ChannelActions(
            channel_ids
                .into_iter()
                .map(|channel_id| {
                    (
                        channel_id,
                        crate::engine::authorization::ChannelAction::ReadHistory,
                    )
                })
                .collect(),
        ))
    } else {
        Ok(DeliveryGuard::Channels(channel_ids))
    }
}

async fn handle_kick(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    if msg.params.len() < 2 {
        return vec![formatter::err_needmoreparams(nick, "KICK")];
    }
    let target_channel = &msg.params[0];
    let target_nick = &msg.params[1];
    let reason = msg.params.get(2).map(|s| s.as_str());

    if !target_channel.starts_with('#') {
        return vec![formatter::err_nosuchchannel(nick, target_channel)];
    }

    let Ok((server_id, channel_name)) =
        resolve_registered_channel(engine, session_id, target_channel).await
    else {
        return vec![formatter::err_nosuchchannel(nick, target_channel)];
    };

    // Resolve channel name → channel_id for channel-scoped permission check
    let channel_id = engine.resolve_channel_id(&server_id, &channel_name).ok();

    let Some(target_user_id) = engine
        .get_session_id_by_nick(target_nick)
        .and_then(|target_session| engine.get_session_user_id(target_session))
    else {
        return vec![formatter::err_nosuchnick(nick, target_nick)];
    };

    match engine
        .kick_member_in_channel(
            session_id,
            &server_id,
            &target_user_id,
            reason,
            channel_id.as_deref(),
        )
        .await
    {
        Ok(()) => vec![],
        Err(e) => {
            // Map permission errors to IRC numeric 482
            if e.contains("permission") || e.contains("Permission") {
                vec![format!(
                    ":{} 482 {} {} :{}",
                    formatter::server_name(),
                    nick,
                    target_channel,
                    e
                )]
            } else {
                vec![format!(
                    ":{} NOTICE {} :KICK failed: {}",
                    formatter::server_name(),
                    nick,
                    e
                )]
            }
        }
    }
}

/// Handle IRC AWAY command: AWAY [:message] / AWAY (no params = back)
async fn handle_away(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let sn = formatter::server_name();
    if let Some(away_msg) = msg.params.first() {
        match engine
            .set_presence(session_id, "idle", Some(away_msg), None)
            .await
        {
            Ok(()) => vec![format!(
                ":{sn} 306 {nick} :You have been marked as being away"
            )],
            Err(e) => vec![format!(":{sn} NOTICE {nick} :AWAY failed: {e}")],
        }
    } else {
        match engine.set_presence(session_id, "online", None, None).await {
            Ok(()) => vec![format!(
                ":{sn} 305 {nick} :You are no longer marked as being away"
            )],
            Err(e) => vec![format!(":{sn} NOTICE {nick} :AWAY failed: {e}")],
        }
    }
}

/// Handle IRC INVITE command: INVITE target #channel
async fn handle_invite(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let sn = formatter::server_name();
    if msg.params.len() < 2 {
        return vec![formatter::err_needmoreparams(nick, "INVITE")];
    }
    let target_nick = &msg.params[0];
    let target_channel = &msg.params[1];

    if !target_channel.starts_with('#') {
        return vec![formatter::err_nosuchchannel(nick, target_channel)];
    }

    let Ok((server_id, channel_name)) =
        resolve_registered_channel(engine, session_id, target_channel).await
    else {
        return vec![formatter::err_nosuchchannel(nick, target_channel)];
    };

    // Resolve target nickname → session_id
    let target_sid = match engine.get_session_id_by_nick(target_nick) {
        Some(sid) => sid,
        None => return vec![formatter::err_nosuchnick(nick, target_nick)],
    };

    // Join target to the channel
    if let Err(e) = engine
        .join_channel(target_sid, &server_id, &channel_name)
        .await
    {
        return vec![format!(":{sn} NOTICE {nick} :INVITE failed: {e}")];
    }

    let irc_channel = commands::to_irc_channel(engine, &server_id, &channel_name);
    vec![format!(":{sn} 341 {nick} {target_nick} {irc_channel}")]
}

/// Handle IRC WHOIS command with channel list and away status.
async fn handle_whois(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> GuardedCommandReplies {
    use crate::engine::user_session::DeliveryGuard;
    let Some(target) = msg.params.first().or(msg.params.get(1)) else {
        return Ok((
            vec![formatter::err_needmoreparams(nick, "WHOIS")],
            DeliveryGuard::ActorCurrent,
        ));
    };
    // Strip leading server param: WHOIS server target → use target
    let target = target.as_str();

    let Some(target_sid) = engine.get_session_id_by_nick(target) else {
        return Ok((
            vec![formatter::err_nosuchnick(nick, target)],
            DeliveryGuard::ActorCurrent,
        ));
    };

    let Some(actor) = engine.get_authenticated_actor(session_id) else {
        return Err(());
    };

    let mut lines = vec![
        formatter::rpl_whoisuser(nick, target),
        formatter::rpl_whoisserver(nick, target),
    ];

    // 319 RPL_WHOISCHANNELS — list channels the target is in
    let mut visible_channels = Vec::new();
    let mut stamps = Vec::new();
    let target_user_id = engine.get_session_user_id(target_sid);
    let mut away_message = None;
    for (server_id, channel_name) in engine.get_session_channels(target_sid) {
        if let Ok((members, stamp)) = engine
            .get_visible_members(&actor, &server_id, &channel_name)
            .await
            && members.iter().any(|member| member.nickname == target)
        {
            visible_channels.push(to_irc_channel(engine, &server_id, &channel_name));
            stamps.push(stamp);
            if away_message.is_none()
                && let Some(target_user_id) = target_user_id.as_deref()
                && let Ok(presences) = engine.get_server_presences(session_id, &server_id).await
                && let Some(presence) = presences.iter().find(|item| {
                    item.user_id == target_user_id && matches!(item.status.as_str(), "idle" | "dnd")
                })
            {
                away_message = Some(
                    presence
                        .custom_status
                        .clone()
                        .unwrap_or_else(|| "Away".into()),
                );
            }
        }
    }
    if !visible_channels.is_empty() {
        lines.push(formatter::rpl_whoischannels(
            nick,
            target,
            &visible_channels.join(" "),
        ));
    }

    // 301 RPL_AWAY — if the target has an away/idle status with a custom message
    if let Some(away_message) = away_message {
        lines.push(formatter::rpl_away(nick, target, &away_message));
    }

    lines.push(formatter::rpl_endofwhois(nick, target));
    let guard = if stamps.is_empty() {
        DeliveryGuard::ActorCurrent
    } else {
        DeliveryGuard::Stamps(stamps)
    };
    Ok((lines, guard))
}

/// Determine the IRC prefix character (@, +, or none) for a user in a server.
/// @ = operator (MANAGE_CHANNELS, KICK_MEMBERS, BAN_MEMBERS, or ADMINISTRATOR)
/// + = voice (MANAGE_MESSAGES but not operator-level)
async fn irc_prefix_for_user(engine: &ChatEngine, server_id: &str, user_id: &str) -> &'static str {
    let perms = engine
        .get_effective_permissions(server_id, None, user_id)
        .await;
    if perms.contains(Permissions::ADMINISTRATOR)
        || perms.contains(Permissions::MANAGE_CHANNELS)
        || perms.contains(Permissions::KICK_MEMBERS)
        || perms.contains(Permissions::BAN_MEMBERS)
    {
        "@"
    } else if perms.contains(Permissions::MANAGE_MESSAGES) {
        "+"
    } else {
        ""
    }
}

/// Handle IRC NAMES command with role-based prefixes (@/+).
async fn handle_names_async(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> GuardedCommandReplies {
    use crate::engine::user_session::DeliveryGuard;
    let Some(channel_param) = msg.params.first() else {
        return Ok((
            vec![formatter::err_needmoreparams(nick, "NAMES")],
            DeliveryGuard::ActorCurrent,
        ));
    };

    let Ok((server_id, channel_name)) =
        resolve_registered_channel(engine, session_id, channel_param).await
    else {
        return Ok((
            vec![formatter::rpl_endofnames(nick, channel_param)],
            DeliveryGuard::ActorCurrent,
        ));
    };
    let irc_channel = to_irc_channel(engine, &server_id, &channel_name);
    let actor = engine.get_authenticated_actor(session_id);

    match actor {
        Some(actor) => match engine
            .get_visible_members(&actor, &server_id, &channel_name)
            .await
        {
            Ok((member_infos, stamp)) => {
                let mut nicks = Vec::with_capacity(member_infos.len());
                for m in &member_infos {
                    let uid = m.user_id.as_deref().unwrap_or("");
                    let prefix = irc_prefix_for_user(engine, &server_id, uid).await;
                    nicks.push(format!("{prefix}{}", m.nickname));
                }
                Ok((
                    vec![
                        formatter::rpl_namreply(nick, &irc_channel, &nicks),
                        formatter::rpl_endofnames(nick, &irc_channel),
                    ],
                    DeliveryGuard::Stamps(vec![stamp]),
                ))
            }
            Err(_) => Err(()),
        },
        None => Err(()),
    }
}

async fn handle_list_async(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> GuardedCommandReplies {
    use crate::engine::user_session::DeliveryGuard;
    let server_alias = msg.params.first().and_then(|pattern| {
        pattern
            .strip_prefix('#')
            .unwrap_or(pattern)
            .strip_suffix("/*")
    });
    let Some(actor) = engine.get_authenticated_actor(session_id) else {
        return Err(());
    };
    let (channels, stamp) = {
        let Ok(server_id) = engine
            .resolve_irc_server_for_actor(&actor, server_alias)
            .await
        else {
            return Ok((
                vec![formatter::rpl_listend(nick)],
                DeliveryGuard::ActorCurrent,
            ));
        };
        match engine
            .list_visible_channels_for_actor(&server_id, &actor)
            .await
        {
            Ok((channels, stamp)) => (
                channels
                    .into_iter()
                    .map(|channel| (server_id.clone(), channel))
                    .collect::<Vec<_>>(),
                stamp,
            ),
            Err(_) => return Err(()),
        }
    };
    let mut replies = Vec::with_capacity(channels.len() + 1);
    for (server_id, channel) in channels {
        replies.push(formatter::rpl_list(
            nick,
            &to_irc_channel(engine, &server_id, &channel.name),
            channel.member_count,
            &channel.topic,
        ));
    }
    replies.push(formatter::rpl_listend(nick));
    Ok((replies, DeliveryGuard::Stamps(vec![stamp])))
}

async fn handle_history_async(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
    caps: &ClientCaps,
) -> GuardedCommandReplies {
    use crate::engine::user_session::DeliveryGuard;
    let Some(channel_param) = msg.params.first() else {
        return Ok((
            vec![formatter::err_needmoreparams(nick, "HISTORY")],
            DeliveryGuard::ActorCurrent,
        ));
    };
    let Some(actor) = engine.get_authenticated_actor(session_id) else {
        return Err(());
    };
    let Ok((server_id, channel_name)) = engine
        .resolve_irc_channel_for_actor(&actor, channel_param)
        .await
    else {
        return Err(());
    };
    let limit = msg
        .params
        .get(1)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(50)
        .clamp(1, 100);
    let Ok((messages, _, stamp)) = engine
        .fetch_history(&server_id, &channel_name, None, limit, &actor)
        .await
    else {
        return Err(());
    };
    let target = to_irc_channel(engine, &server_id, &channel_name);
    let replies = messages
        .into_iter()
        .map(|message| {
            let content = message.content.replace(['\r', '\n'], " ");
            let tag_prefix = build_history_tag_prefix(caps, &message.id, &message.timestamp);
            format!(
                "{}:{}!{}@{} PRIVMSG {} :{}",
                tag_prefix,
                message.from,
                message.from,
                formatter::server_name(),
                target,
                content
            )
        })
        .collect();
    Ok((replies, DeliveryGuard::Stamps(vec![stamp])))
}

/// Handle IRC WHO command with role-based prefixes (@/+).
async fn handle_who_async(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> GuardedCommandReplies {
    use crate::engine::user_session::DeliveryGuard;
    let Some(target) = msg.params.first() else {
        return Ok((
            vec![formatter::err_needmoreparams(nick, "WHO")],
            DeliveryGuard::ActorCurrent,
        ));
    };

    let mut replies = Vec::new();

    if target.starts_with('#') {
        let Ok((server_id, channel_name)) =
            resolve_registered_channel(engine, session_id, target).await
        else {
            return Ok((
                vec![format!(
                    ":{} {} {} {} :End of /WHO list",
                    formatter::server_name(),
                    super::numerics::RPL_ENDOFWHO,
                    nick,
                    target,
                )],
                DeliveryGuard::ActorCurrent,
            ));
        };
        let irc_channel = to_irc_channel(engine, &server_id, &channel_name);

        let Some(actor) = engine.get_authenticated_actor(session_id) else {
            return Err(());
        };
        let Ok((members, stamp)) = engine
            .get_visible_members(&actor, &server_id, &channel_name)
            .await
        else {
            return Err(());
        };
        for member in &members {
            let uid = member.user_id.as_deref().unwrap_or("");
            let prefix = irc_prefix_for_user(engine, &server_id, uid).await;
            // RFC 2812: 352 <requestor> <channel> <user> <host> <server> <nick> <H|G>[*][@|+] :<hopcount> <realname>
            replies.push(format!(
                ":{} {} {} {} {} {} {} {} H{prefix} :0 {}",
                formatter::server_name(),
                super::numerics::RPL_WHOREPLY,
                nick,
                irc_channel,
                member.nickname,          // user (ident)
                formatter::server_name(), // host
                formatter::server_name(), // server
                member.nickname,          // nick
                member.nickname,          // realname
            ));
        }

        replies.push(format!(
            ":{} {} {} {} :End of /WHO list",
            formatter::server_name(),
            super::numerics::RPL_ENDOFWHO,
            nick,
            irc_channel,
        ));
        return Ok((replies, DeliveryGuard::Stamps(vec![stamp])));
    } else {
        replies.push(format!(
            ":{} {} {} {} :End of /WHO list",
            formatter::server_name(),
            super::numerics::RPL_ENDOFWHO,
            nick,
            target,
        ));
    }

    Ok((replies, DeliveryGuard::ActorCurrent))
}

fn escape_ircv3_tag_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            ';' => escaped.push_str("\\:"),
            ' ' => escaped.push_str("\\s"),
            '\\' => escaped.push_str("\\\\"),
            '\r' => escaped.push_str("\\r"),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn build_history_tag_prefix(
    caps: &ClientCaps,
    message_id: &crate::engine::ids::MessageId,
    timestamp: &chrono::DateTime<chrono::Utc>,
) -> String {
    let mut tags = Vec::new();
    if caps.server_time {
        tags.push(format!("time={}", timestamp.to_rfc3339()));
    }
    if caps.message_tags {
        tags.push(format!(
            "msgid={}",
            escape_ircv3_tag_value(message_id.as_str())
        ));
    }
    if tags.is_empty() {
        String::new()
    } else {
        format!("@{} ", tags.join(";"))
    }
}

/// Build an IRCv3 tag prefix string based on event metadata and negotiated caps.
fn build_tag_prefix(caps: &ClientCaps, event: &ChatEvent) -> String {
    let mut tags = Vec::new();
    if caps.server_time {
        // Extract timestamp from events that have one
        if let ChatEvent::Message { timestamp, .. } = event {
            tags.push(format!(
                "time={}",
                timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ")
            ));
        }
    }
    if caps.message_tags {
        // Attach message ID where available
        if let ChatEvent::Message { id, .. } = event {
            tags.push(format!("msgid={}", escape_ircv3_tag_value(id.as_str())));
        }
    }
    if tags.is_empty() {
        String::new()
    } else {
        format!("@{} ", tags.join(";"))
    }
}

/// Convert a ChatEvent to IRC protocol lines for a specific recipient.
/// Uses the engine to translate (server_id, channel_name) to IRC format.
fn event_to_irc_lines(
    engine: &ChatEngine,
    my_nick: &str,
    event: &ChatEvent,
    caps: &ClientCaps,
) -> Vec<String> {
    let tag_prefix = build_tag_prefix(caps, event);
    let mut lines = event_to_irc_lines_inner(engine, my_nick, event);
    if !tag_prefix.is_empty() {
        for line in &mut lines {
            line.insert_str(0, &tag_prefix);
        }
    }
    lines
}

/// Inner function that produces raw IRC lines without tags.
fn event_to_irc_lines_inner(engine: &ChatEngine, my_nick: &str, event: &ChatEvent) -> Vec<String> {
    match event {
        ChatEvent::Message {
            server_id,
            from,
            target,
            content,
            reply_to,
            attachments,
            ..
        } => {
            let irc_target = if target.starts_with('#') {
                let sid = server_id.as_deref().unwrap_or(DEFAULT_SERVER_ID);
                to_irc_channel(engine, sid, target)
            } else {
                target.clone()
            };
            // Build display content with reply context prefix
            let display = if let Some(reply) = reply_to {
                format!("[re: {} \"{}\"] {}", reply.from, reply.content_preview, content)
            } else {
                content.clone()
            };
            let mut lines = Vec::new();
            // Convert /me prefix to CTCP ACTION
            if let Some(action) = display.strip_prefix("/me ") {
                lines.push(formatter::ctcp_action(from, &irc_target, action));
            } else {
                lines.push(formatter::privmsg(from, &irc_target, &display));
            }
            // Append attachment URLs as separate messages
            if let Some(atts) = attachments {
                for att in atts {
                    lines.push(formatter::privmsg(from, &irc_target, &att.url));
                }
            }
            lines
        }
        ChatEvent::Join {
            nickname,
            server_id,
            channel,
            ..
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![formatter::join(nickname, &irc_channel)]
        }
        ChatEvent::Part {
            nickname,
            server_id,
            channel,
            reason,
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![formatter::part(nickname, &irc_channel, reason.as_deref())]
        }
        ChatEvent::Quit { nickname, reason } => {
            vec![formatter::quit(nickname, reason.as_deref())]
        }
        ChatEvent::TopicChange {
            server_id,
            channel,
            set_by,
            topic,
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![formatter::topic_change(set_by, &irc_channel, topic)]
        }
        ChatEvent::NickChange { old_nick, new_nick } => {
            vec![formatter::nick_change(old_nick, new_nick)]
        }
        ChatEvent::Names {
            server_id,
            channel,
            members,
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            let owner_id = engine.get_server_owner_id(server_id);
            let nicks: Vec<String> = members
                .iter()
                .map(|m| {
                    // Prefix server owner with @ (operator)
                    if owner_id.as_deref() == m.user_id.as_deref() && m.user_id.is_some() {
                        format!("@{}", m.nickname)
                    } else {
                        m.nickname.clone()
                    }
                })
                .collect();
            vec![
                formatter::rpl_namreply(my_nick, &irc_channel, &nicks),
                formatter::rpl_endofnames(my_nick, &irc_channel),
            ]
        }
        ChatEvent::Topic {
            server_id,
            channel,
            topic,
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            if topic.is_empty() {
                vec![formatter::rpl_notopic(my_nick, &irc_channel)]
            } else {
                vec![formatter::rpl_topic(my_nick, &irc_channel, topic)]
            }
        }
        ChatEvent::ServerNotice { message } => {
            vec![format!(
                ":{} NOTICE {} :{}",
                formatter::server_name(),
                my_nick,
                message
            )]
        }
        ChatEvent::Error { code, message } => {
            vec![format!(
                ":{} NOTICE {} :[{}] {}",
                formatter::server_name(),
                my_nick,
                code,
                message
            )]
        }
        // Message edit: send a NOTICE indicating the edit
        ChatEvent::MessageEdit {
            server_id, channel, ..
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![format!(
                ":{} NOTICE {} :* A message was edited in {}",
                formatter::server_name(),
                my_nick,
                irc_channel
            )]
        }
        // Message delete: send a NOTICE indicating the deletion
        ChatEvent::MessageDelete {
            server_id, channel, ..
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![format!(
                ":{} NOTICE {} :* A message was deleted in {}",
                formatter::server_name(),
                my_nick,
                irc_channel
            )]
        }
        // MessageAck is WS-only (sender-only event)
        ChatEvent::MessageAck { .. } => vec![],
        // Reactions: show as a PRIVMSG action from the reacting user
        ChatEvent::ReactionAdd {
            server_id,
            channel,
            nickname,
            emoji,
            ..
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![formatter::ctcp_action(nickname, &irc_channel, &format!("reacted with {emoji}"))]
        }
        ChatEvent::ReactionRemove {
            server_id,
            channel,
            nickname,
            emoji,
            ..
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![formatter::ctcp_action(nickname, &irc_channel, &format!("removed reaction {emoji}"))]
        }
        // Typing indicators are not sent to IRC
        ChatEvent::TypingStart { .. } => vec![],
        // Embeds are WebSocket-only (rich previews don't map to IRC)
        ChatEvent::MessageEmbed { .. } => vec![],
        // Phase 5: Pinning — send NOTICEs for pin/unpin actions
        ChatEvent::MessagePin {
            server_id,
            channel,
            pin,
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![format!(
                ":{} NOTICE {} :\u{1f4cc} {} pinned a message from {}",
                formatter::server_name(),
                irc_channel,
                pin.pinned_by,
                pin.from
            )]
        }
        ChatEvent::MessageUnpin {
            server_id, channel, ..
        } => {
            let irc_channel = to_irc_channel(engine, server_id, channel);
            vec![format!(
                ":{} NOTICE {} :\u{1f4cc} Message unpinned in {}",
                formatter::server_name(),
                irc_channel,
                irc_channel
            )]
        }
        // Phase 5: Threads — send NOTICE for new thread creation and updates
        ChatEvent::ThreadCreate {
            server_id,
            parent_channel,
            thread,
        } => {
            let irc_channel = to_irc_channel(engine, server_id, parent_channel);
            vec![format!(
                ":{} NOTICE {} :\u{1f9f5} New thread: {}",
                formatter::server_name(),
                irc_channel,
                thread.name
            )]
        }
        ChatEvent::ThreadUpdate {
            server_id: _,
            thread,
        } => {
            // ThreadUpdate has no channel field; use server_id for context
            let action = if thread.archived { "archived" } else { "unarchived" };
            vec![format!(
                ":{} NOTICE {} :\u{1f9f5} Thread \"{}\" was {}",
                formatter::server_name(),
                my_nick,
                thread.name,
                action
            )]
        }
        // Phase 6: Moderation — kick and ban get NOTICEs, rest are WS-only
        ChatEvent::MemberKick { server_id: _, user_id: _, kicked_by, reason } => {
            let reason_text = reason.as_deref().unwrap_or("No reason given");
            vec![format!(
                ":{} NOTICE {} :{} kicked a member: {}",
                formatter::server_name(),
                my_nick,
                kicked_by,
                reason_text
            )]
        }
        ChatEvent::MemberBan { server_id: _, user_id: _, banned_by, reason } => {
            let reason_text = reason.as_deref().unwrap_or("No reason given");
            vec![format!(
                ":{} NOTICE {} :{} banned a member: {}",
                formatter::server_name(),
                my_nick,
                banned_by,
                reason_text
            )]
        }
        ChatEvent::MemberUnban { .. } => vec![],
        ChatEvent::MemberTimeout { .. } => vec![],
        ChatEvent::SlowModeUpdate { .. } => vec![],
        ChatEvent::NsfwUpdate { .. } => vec![],
        ChatEvent::BulkMessageDelete { .. } => vec![],
        ChatEvent::AuditLogEntries { .. } => vec![],
        ChatEvent::BanList { .. } => vec![],
        ChatEvent::AutomodRuleList { .. } => vec![],
        ChatEvent::AutomodRuleUpdate { .. } => vec![],
        ChatEvent::AutomodRuleDelete { .. } => vec![],
        // These events are WebSocket-specific and don't map to IRC
        ChatEvent::ChannelList { .. }
        | ChatEvent::History { .. }
        | ChatEvent::ServerList { .. }
        | ChatEvent::UnreadCounts { .. }
        | ChatEvent::RoleList { .. }
        | ChatEvent::RoleUpdate { .. }
        | ChatEvent::RoleDelete { .. }
        | ChatEvent::MemberRoleUpdate { .. }
        | ChatEvent::ChannelPermissionOverrideList { .. }
        | ChatEvent::CategoryList { .. }
        | ChatEvent::CategoryUpdate { .. }
        | ChatEvent::CategoryDelete { .. }
        | ChatEvent::ChannelReorder { .. }
        | ChatEvent::PresenceUpdate { .. }
        | ChatEvent::PresenceList { .. }
        | ChatEvent::OwnPresence { .. }
        | ChatEvent::UserProfile { .. }
        | ChatEvent::ServerNicknameUpdate { .. }
        | ChatEvent::NotificationSettings { .. }
        | ChatEvent::SearchResults { .. }
        | ChatEvent::PinnedMessages { .. }
        | ChatEvent::ThreadList { .. }
        | ChatEvent::ForumTagList { .. }
        | ChatEvent::ForumTagUpdate { .. }
        | ChatEvent::ForumTagDelete { .. }
        | ChatEvent::BookmarkList { .. }
        | ChatEvent::BookmarkAdd { .. }
        | ChatEvent::BookmarkRemove { .. }
        | ChatEvent::InviteList { .. }
        | ChatEvent::InviteCreate { .. }
        | ChatEvent::InviteDelete { .. }
        | ChatEvent::EventList { .. }
        | ChatEvent::EventUpdate { .. }
        | ChatEvent::EventDelete { .. }
        | ChatEvent::EventRsvpList { .. }
        | ChatEvent::ServerCommunity { .. }
        | ChatEvent::DiscoverServers { .. }
        | ChatEvent::ChannelFollowList { .. }
        | ChatEvent::ChannelFollowCreate { .. }
        | ChatEvent::ChannelFollowDelete { .. }
        | ChatEvent::AnnouncementPublished { .. }
        | ChatEvent::TemplateList { .. }
        | ChatEvent::TemplateUpdate { .. }
        | ChatEvent::TemplateDelete { .. }
        | ChatEvent::TemplateInstantiated { .. }
        // Phase 8: Integrations (web-only)
        | ChatEvent::SyncSnapshot { .. }
        | ChatEvent::ReplayBatch { .. }
        | ChatEvent::DurableEvent { .. }
        | ChatEvent::DirectConversationList { .. }
        | ChatEvent::ResyncRequired { .. }
        | ChatEvent::CommandError { .. }
        | ChatEvent::CommandCommitted { .. }
        | ChatEvent::WebhookList { .. }
        | ChatEvent::WebhookUpdate { .. }
        | ChatEvent::WebhookDelete { .. }
        | ChatEvent::SlashCommandList { .. }
        | ChatEvent::SlashCommandUpdate { .. }
        | ChatEvent::SlashCommandDelete { .. }
        | ChatEvent::InteractionCreate { .. }
        | ChatEvent::InteractionResponse { .. }
        | ChatEvent::InteractionInvoked { .. }
        | ChatEvent::LifecycleCommandSucceeded { .. }
        | ChatEvent::BotAccountList { .. }
        | ChatEvent::BotCredentialCreated { .. }
        | ChatEvent::BotTokenList { .. }
        | ChatEvent::OAuth2AppList { .. }
        | ChatEvent::OAuth2AppUpdate { .. }
        | ChatEvent::BlueskyProfileSync { .. }
        | ChatEvent::BlueskyShareResult { .. }
        | ChatEvent::ServerAvatarUpdate { .. }
        | ChatEvent::ThreadTagUpdate { .. }
        | ChatEvent::ServerLimits { .. } => vec![],
    }
}

fn send_line(out: &Outbound, line: &str) {
    send_line_guarded(out, line, None);
}

fn sanitize_outbound_line(line: &str) -> String {
    line.replace(['\r', '\n', '\0'], " ")
}

fn send_line_guarded(
    out: &Outbound,
    line: &str,
    guard: Option<crate::engine::user_session::DeliveryGuard>,
) {
    let line = line.to_string();
    let bytes = line.len();
    if out
        .queued_bytes
        .fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= MAX_OUTBOUND_BYTES)
            },
        )
        .is_err()
    {
        out.failed.cancel();
        return;
    }
    if out.tx.try_send(OutboundLine { line, guard }).is_err() {
        out.queued_bytes
            .fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
        out.failed.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::events::{MemberInfo, PinnedMessageInfo, ThreadInfo};
    use crate::engine::ids::MessageId;
    use chrono::Utc;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use uuid::Uuid;

    /// Create a minimal explicit in-memory harness for projection unit tests.
    fn test_engine() -> Arc<ChatEngine> {
        Arc::new(ChatEngine::test_harness(4000, 100))
    }

    /// Test helper — calls the inner (tag-free) event formatter.
    fn event_to_irc_lines(engine: &ChatEngine, my_nick: &str, event: &ChatEvent) -> Vec<String> {
        event_to_irc_lines_inner(engine, my_nick, event)
    }

    #[test]
    fn ircv3_message_tags_preserve_historical_opaque_ids() {
        let caps = ClientCaps {
            server_time: true,
            message_tags: true,
            sasl: false,
        };
        let timestamp = chrono::DateTime::parse_from_rfc3339("2024-01-02T03:04:06.654321-05:00")
            .unwrap()
            .with_timezone(&Utc);
        let id = MessageId::from_stored("  legacy;message\\id  ").unwrap();
        assert_eq!(
            build_history_tag_prefix(&caps, &id, &timestamp),
            "@time=2024-01-02T08:04:06.654321+00:00;msgid=\\s\\slegacy\\:message\\\\id\\s\\s "
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn heartbeat_accepts_exact_pong_then_disconnects_after_a_missed_probe() {
        let db = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&db).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('heartbeat-user','heartbeat')")
            .execute(&db)
            .await
            .unwrap();
        let auth = AuthService::new(db.clone(), "heartbeat-secret".into(), 1);
        let web_actor = auth.issue_web_session("heartbeat-user").await.unwrap().1;
        let token = auth
            .issue_irc_token(web_actor.user_id(), Some("heartbeat test"))
            .await
            .unwrap();
        let engine = Arc::new(ChatEngine::new(
            db.clone(),
            auth.clone(),
            "heartbeat-secret",
            4000,
            100,
        ));
        let (server, client) = tokio::io::duplex(16 * 1024);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(handle_irc_connection_with_timing(
            server,
            "heartbeat-peer".into(),
            engine,
            db,
            auth,
            cancel,
            Duration::from_millis(100),
        ));
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader);
        writer
            .write_all(
                format!(
                    "PASS {}\r\nNICK heartbeat\r\nUSER heartbeat 0 * :Heartbeat\r\n",
                    token.secret
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let first_nonce = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if let Some(nonce) = line.trim_end().strip_prefix("PING :") {
                    break nonce.to_owned();
                }
            }
        })
        .await
        .expect("registered client did not receive heartbeat probe");
        writer
            .write_all(format!("PONG :{first_nonce}\r\n").as_bytes())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line.starts_with("PING :") {
                    break;
                }
            }
        })
        .await
        .expect("exact PONG did not keep the connection alive for the next probe");

        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("client remained connected after missing the next heartbeat")
            .unwrap();
    }

    // ── Message event ──

    #[test]
    fn test_message_event_to_channel() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Message {
                id: Uuid::new_v4().into(),
                server_id: Some(DEFAULT_SERVER_ID.to_string()),
                conversation_id: None,
                from: "alice".into(),
                target: "#general".into(),
                content: "Hello world".into(),
                timestamp: Utc::now(),
                avatar_url: None,
                reply_to: None,
                attachments: None,
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("PRIVMSG #general :Hello world"));
        assert!(lines[0].starts_with(":alice!"));
    }

    #[test]
    fn test_message_event_dm() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "bob",
            &ChatEvent::Message {
                id: Uuid::new_v4().into(),
                server_id: None,
                conversation_id: None,
                from: "alice".into(),
                target: "bob".into(),
                content: "Hey there".into(),
                timestamp: Utc::now(),
                avatar_url: None,
                reply_to: None,
                attachments: None,
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("PRIVMSG bob :Hey there"));
    }

    // ── Join/Part/Quit/Nick events ──

    #[test]
    fn test_join_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Join {
                nickname: "alice".into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                avatar_url: None,
                user_id: Some("alice-id".into()),
                server_avatar_url: None,
                role_ids: Vec::new(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("JOIN #general"));
        assert!(lines[0].starts_with(":alice!"));
    }

    #[test]
    fn test_part_event_with_reason() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Part {
                nickname: "bob".into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                reason: Some("goodbye".into()),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("PART #general"));
        assert!(lines[0].contains("goodbye"));
    }

    #[test]
    fn test_part_event_no_reason() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Part {
                nickname: "bob".into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                reason: None,
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("PART #general"));
    }

    #[test]
    fn test_quit_event_with_reason() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Quit {
                nickname: "alice".into(),
                reason: Some("Leaving".into()),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("QUIT"));
        assert!(lines[0].contains("Leaving"));
    }

    #[test]
    fn test_quit_event_no_reason() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Quit {
                nickname: "alice".into(),
                reason: None,
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("QUIT"));
    }

    #[test]
    fn test_nick_change_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::NickChange {
                old_nick: "alice".into(),
                new_nick: "alice_".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("NICK"));
        assert!(lines[0].contains("alice_"));
    }

    // ── Topic events ──

    #[test]
    fn test_topic_change_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::TopicChange {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                set_by: "alice".into(),
                topic: "New topic".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("TOPIC #general"));
        assert!(lines[0].contains("New topic"));
    }

    #[test]
    fn test_topic_event_with_content() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Topic {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#dev".into(),
                topic: "Development chat".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("#dev"));
        assert!(lines[0].contains("Development chat"));
    }

    #[test]
    fn test_topic_event_empty() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Topic {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#dev".into(),
                topic: "".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        // Empty topic produces RPL_NOTOPIC
        assert!(lines[0].contains("331"));
    }

    // ── Names event ──

    #[test]
    fn test_names_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Names {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                members: vec![
                    MemberInfo {
                        nickname: "alice".into(),
                        avatar_url: None,
                        status: None,
                        custom_status: None,
                        status_emoji: None,
                        user_id: None,
                        server_avatar_url: None,
                        role_ids: Vec::new(),
                    },
                    MemberInfo {
                        nickname: "bob".into(),
                        avatar_url: None,
                        status: None,
                        custom_status: None,
                        status_emoji: None,
                        user_id: None,
                        server_avatar_url: None,
                        role_ids: Vec::new(),
                    },
                ],
            },
        );
        // Names produces RPL_NAMREPLY + RPL_ENDOFNAMES
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("353"));
        assert!(lines[0].contains("alice"));
        assert!(lines[0].contains("bob"));
        assert!(lines[1].contains("366"));
    }

    // ── ServerNotice / Error events ──

    #[test]
    fn test_server_notice_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::ServerNotice {
                message: "Welcome to Concord".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("NOTICE viewer :Welcome to Concord"));
    }

    #[test]
    fn test_error_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Error {
                code: "NOT_FOUND".into(),
                message: "Channel not found".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("NOTICE viewer"));
        assert!(lines[0].contains("[NOT_FOUND]"));
        assert!(lines[0].contains("Channel not found"));
    }

    // ── MessageEdit / MessageDelete events ──

    #[test]
    fn test_message_edit_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MessageEdit {
                id: Uuid::new_v4().into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                content: "edited content".into(),
                edited_at: Utc::now(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("NOTICE viewer"));
        assert!(lines[0].contains("edited"));
        assert!(lines[0].contains("#general"));
    }

    #[test]
    fn test_message_delete_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MessageDelete {
                id: Uuid::new_v4().into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("NOTICE viewer"));
        assert!(lines[0].contains("deleted"));
        assert!(lines[0].contains("#general"));
    }

    // ── Reaction events ──

    #[test]
    fn test_reaction_add_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::ReactionAdd {
                message_id: Uuid::new_v4().into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                user_id: "uid1".into(),
                nickname: "alice".into(),
                emoji: "\u{1f44d}".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("alice"));
        assert!(lines[0].contains("\u{1f44d}"));
        assert!(lines[0].contains("#general"));
    }

    #[test]
    fn test_reaction_remove_event_formats_action() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::ReactionRemove {
                message_id: Uuid::new_v4().into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                user_id: "uid1".into(),
                nickname: "alice".into(),
                emoji: "\u{1f44d}".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("ACTION"));
        assert!(lines[0].contains("removed reaction"));
    }

    // ── Events that produce no IRC output ──

    #[test]
    fn test_typing_start_is_silent() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::TypingStart {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                nickname: "alice".into(),
            },
        );
        assert!(lines.is_empty());
    }

    #[test]
    fn test_message_embed_is_silent() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MessageEmbed {
                message_id: Uuid::new_v4().into(),
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                embeds: vec![],
            },
        );
        assert!(lines.is_empty());
    }

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

    // ── Phase 5: Pin/Thread events ──

    #[test]
    fn test_message_pin_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MessagePin {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                pin: PinnedMessageInfo {
                    id: "pin-1".into(),
                    message_id: "msg-1".into(),
                    channel_id: "ch-1".into(),
                    pinned_by: "alice".into(),
                    pinned_at: "2025-01-01".into(),
                    from: "bob".into(),
                    content: "Important msg".into(),
                    timestamp: "2025-01-01".into(),
                },
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("alice"));
        assert!(lines[0].contains("pinned"));
        assert!(lines[0].contains("bob"));
    }

    #[test]
    fn test_message_unpin_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MessageUnpin {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                message_id: "msg-1".into(),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("unpinned"));
    }

    #[test]
    fn test_thread_create_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::ThreadCreate {
                server_id: DEFAULT_SERVER_ID.into(),
                parent_channel: "#general".into(),
                thread: ThreadInfo {
                    id: "thread-1".into(),
                    name: "Discussion".into(),
                    channel_type: "public_thread".into(),
                    parent_message_id: None,
                    creator_user_id: None,
                    archived: false,
                    state_version: 1,
                    tags_version: 1,
                    tag_ids: Vec::new(),
                    auto_archive_minutes: 1440,
                    message_count: 0,
                    created_at: "2025-01-01".into(),
                },
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Discussion"));
        assert!(lines[0].contains("thread"));
    }

    #[test]
    fn test_thread_update_archived() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::ThreadUpdate {
                server_id: DEFAULT_SERVER_ID.into(),
                thread: ThreadInfo {
                    id: "thread-1".into(),
                    name: "Old thread".into(),
                    channel_type: "public_thread".into(),
                    parent_message_id: None,
                    creator_user_id: None,
                    archived: true,
                    state_version: 2,
                    tags_version: 1,
                    tag_ids: Vec::new(),
                    auto_archive_minutes: 1440,
                    message_count: 5,
                    created_at: "2025-01-01".into(),
                },
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("archived"));
        assert!(lines[0].contains("Old thread"));
    }

    #[test]
    fn test_thread_update_unarchived() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::ThreadUpdate {
                server_id: DEFAULT_SERVER_ID.into(),
                thread: ThreadInfo {
                    id: "thread-1".into(),
                    name: "Revived thread".into(),
                    channel_type: "public_thread".into(),
                    parent_message_id: None,
                    creator_user_id: None,
                    archived: false,
                    state_version: 3,
                    tags_version: 1,
                    tag_ids: Vec::new(),
                    auto_archive_minutes: 1440,
                    message_count: 10,
                    created_at: "2025-01-01".into(),
                },
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("unarchived"));
    }

    // ── Phase 6: Moderation events ──

    #[test]
    fn test_member_kick_event_with_reason() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MemberKick {
                server_id: DEFAULT_SERVER_ID.into(),
                user_id: "uid1".into(),
                kicked_by: "admin".into(),
                reason: Some("Rule violation".into()),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("admin"));
        assert!(lines[0].contains("kicked"));
        assert!(lines[0].contains("Rule violation"));
    }

    #[test]
    fn test_member_kick_event_no_reason() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MemberKick {
                server_id: DEFAULT_SERVER_ID.into(),
                user_id: "uid1".into(),
                kicked_by: "admin".into(),
                reason: None,
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("No reason given"));
    }

    #[test]
    fn test_member_ban_event() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MemberBan {
                server_id: DEFAULT_SERVER_ID.into(),
                user_id: "uid1".into(),
                banned_by: "admin".into(),
                reason: Some("Spam".into()),
            },
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("banned"));
        assert!(lines[0].contains("Spam"));
    }

    #[test]
    fn test_member_unban_is_silent() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::MemberUnban {
                server_id: DEFAULT_SERVER_ID.into(),
                user_id: "uid1".into(),
            },
        );
        assert!(lines.is_empty());
    }

    #[test]
    fn test_slow_mode_update_is_silent() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::SlowModeUpdate {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                seconds: 5,
            },
        );
        assert!(lines.is_empty());
    }

    #[test]
    fn test_bulk_message_delete_is_silent() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::BulkMessageDelete {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                message_ids: vec!["msg-1".into(), "msg-2".into()],
            },
        );
        assert!(lines.is_empty());
    }

    // ── WebSocket-only events produce no IRC output ──

    #[test]
    fn test_ws_only_events_are_silent() {
        let engine = test_engine();

        let ws_events: Vec<ChatEvent> = vec![
            ChatEvent::History {
                server_id: DEFAULT_SERVER_ID.into(),
                channel: "#general".into(),
                messages: vec![],
                has_more: false,
            },
            ChatEvent::ServerList { servers: vec![] },
            ChatEvent::RoleList {
                server_id: DEFAULT_SERVER_ID.into(),
                version: 0,
                roles: vec![],
                member_roles: Some(vec![]),
            },
            ChatEvent::CategoryList {
                server_id: DEFAULT_SERVER_ID.into(),
                categories: vec![],
            },
            ChatEvent::PresenceList {
                server_id: DEFAULT_SERVER_ID.into(),
                presences: vec![],
            },
            ChatEvent::BookmarkList { bookmarks: vec![] },
            ChatEvent::InviteList {
                server_id: DEFAULT_SERVER_ID.into(),
                invites: vec![],
            },
            ChatEvent::TemplateList {
                server_id: DEFAULT_SERVER_ID.into(),
                templates: vec![],
            },
            ChatEvent::WebhookList {
                server_id: DEFAULT_SERVER_ID.into(),
                webhooks: vec![],
            },
        ];

        for event in &ws_events {
            let lines = event_to_irc_lines(&engine, "viewer", event);
            assert!(
                lines.is_empty(),
                "Expected no IRC output for {:?} but got {:?}",
                std::mem::discriminant(event),
                lines
            );
        }
    }

    // ── send_line helper test ──

    #[test]
    fn test_send_line_sends_to_channel() {
        let (tx, mut rx) = mpsc::channel::<OutboundLine>(1024);
        let out = Outbound {
            tx,
            failed: CancellationToken::new(),
            actor: Arc::new(std::sync::RwLock::new(None)),
            queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        send_line(&out, "PRIVMSG #test :Hello");
        let received = rx.try_recv().unwrap();
        assert_eq!(received.line, "PRIVMSG #test :Hello");
        assert!(received.guard.is_none());
    }

    #[test]
    fn final_outbound_boundary_replaces_line_breaks_and_nul() {
        assert_eq!(
            sanitize_outbound_line(":alice PRIVMSG #general :one\r\nINJECT\0tail"),
            ":alice PRIVMSG #general :one  INJECT tail"
        );
    }

    #[test]
    fn test_send_line_closed_channel_does_not_panic() {
        let (tx, rx) = mpsc::channel::<OutboundLine>(1024);
        drop(rx); // Close the receiver
        let failed = CancellationToken::new();
        let out = Outbound {
            tx,
            failed: failed.clone(),
            actor: Arc::new(std::sync::RwLock::new(None)),
            queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        send_line(&out, "PRIVMSG #test :Hello");
        assert!(failed.is_cancelled());
    }

    #[test]
    fn test_send_line_full_channel_marks_transport_failed() {
        let (tx, _rx) = mpsc::channel::<OutboundLine>(1);
        let failed = CancellationToken::new();
        let out = Outbound {
            tx,
            failed: failed.clone(),
            actor: Arc::new(std::sync::RwLock::new(None)),
            queued_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        send_line(&out, "first");
        send_line(&out, "overflow");
        assert!(failed.is_cancelled());
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
}
