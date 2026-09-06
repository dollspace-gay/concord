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

    // Bounded channel for outbound lines. Saturation marks the transport failed so
    // a slow client cannot silently lose events while retaining an engine session.
    let (out_tx, out_rx) = mpsc::channel::<OutboundLine>(MAX_OUTBOUND_DESCRIPTORS);
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

    let write_handle = tokio::spawn(
        writer::Writer {
            failed: transport_failed.clone(),
            authority_failed: authority_failed.clone(),
            cancel: cancel.clone(),
            auth: auth.clone(),
            engine: engine.clone(),
            outbound_actor,
            queued_bytes,
        }
        .run(writer, out_rx),
    );

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

            if registration::register(
                registration::Registration {
                    engine: &engine,
                    auth: &auth,
                    out: &out,
                    authority_failed: &authority_failed,
                    state: &mut state,
                    caps: &mut caps,
                    event_rx: &mut event_rx,
                    credential_lease: &mut credential_lease,
                    credential_expiry: &mut credential_expiry,
                },
                msg,
            )
            .await
            .is_break()
            {
                break;
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

#[cfg(test)]
mod tests;

mod command_handlers;
mod outbound;
mod projection;
mod queries;
use command_handlers::handle_away;
use command_handlers::handle_invite;
use command_handlers::handle_kick;
use command_handlers::handle_whois;
use command_handlers::irc_command_delivery_guard;
use command_handlers::resolve_registered_channel;
use outbound::sanitize_outbound_line;
use outbound::send_line;
use outbound::send_line_guarded;
use projection::build_history_tag_prefix;
use projection::event_to_irc_lines;
use queries::handle_history_async;
use queries::handle_list_async;
use queries::handle_names_async;
use queries::handle_who_async;

mod registration;
mod writer;
