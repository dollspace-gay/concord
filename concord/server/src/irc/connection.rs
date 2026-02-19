use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Maximum bytes per IRC line (RFC 2812 says 512; we allow 4096 for safety).
const MAX_LINE_LENGTH: usize = 4096;
/// Idle timeout — disconnect clients that send nothing for 5 minutes.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
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

use crate::auth::token::verify_irc_token;
use crate::db::queries::{presence, users};
use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::{ChatEvent, SessionId};
use crate::engine::user_session::Protocol;

use super::commands::{self, to_irc_channel};
use super::formatter;
use super::parser::IrcMessage;

/// Read a line from the IRC connection, capped at MAX_LINE_LENGTH bytes.
/// Returns Ok(0) on EOF, Ok(n) on success, Err on I/O error or line too long.
async fn read_bounded_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    buf: &mut String,
) -> std::io::Result<usize> {
    // Fill the internal buffer and check for a newline within MAX_LINE_LENGTH
    loop {
        let available = reader.buffer();
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            // Found newline within buffered data
            let line_bytes = &available[..=pos];
            let line = String::from_utf8_lossy(line_bytes).into_owned();
            let len = line_bytes.len();
            buf.push_str(&line);
            reader.consume(len);
            return Ok(len);
        }
        if available.len() >= MAX_LINE_LENGTH {
            // Too long without a newline — discard and signal error
            let discard_len = available.len();
            reader.consume(discard_len);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IRC line exceeds maximum length",
            ));
        }
        // Need more data
        let filled = reader.fill_buf().await?;
        if filled.is_empty() {
            return Ok(0); // EOF
        }
    }
}

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
    Registered { session_id: SessionId, nick: String },
}

/// Handle a single IRC client connection from accept to close.
/// Accepts any stream implementing AsyncRead + AsyncWrite (plain TCP or TLS).
pub async fn handle_irc_connection<S>(
    stream: S,
    peer: String,
    engine: Arc<ChatEngine>,
    db: SqlitePool,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    info!(%peer, "IRC client connected");

    let (reader, writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut writer = writer;

    // Channel for outbound lines (from event loop and command handlers)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    // Spawn writer task
    let write_handle = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            let data = format!("{}\r\n", line);
            if writer.write_all(data.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut state = RegState::Unregistered {
        pass: None,
        nick: None,
        user_received: false,
    };

    let mut line_buf = String::new();
    let mut event_rx: Option<mpsc::Receiver<ChatEvent>> = None;
    let mut cmd_rate = CommandRateLimit::new();
    let mut caps = ClientCaps::default();

    loop {
        // When registered, also select on engine events
        if let Some(ref mut rx) = event_rx {
            tokio::select! {
                result = tokio::time::timeout(IDLE_TIMEOUT, read_bounded_line(&mut reader, &mut line_buf)) => {
                    match result {
                        Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break, // EOF, error, or timeout
                        Ok(Ok(_)) => {}
                    }

                    let line = line_buf.trim_end().to_string();
                    line_buf.clear();

                    if line.is_empty() {
                        continue;
                    }

                    // Enforce per-connection command rate limit
                    if !cmd_rate.check() {
                        warn!(%peer, "IRC command rate limited");
                        continue;
                    }

                    if let RegState::Registered { ref session_id, ref nick } = state {
                        let msg = match IrcMessage::parse(&line) {
                            Ok(m) => m,
                            Err(_) => continue,
                        };

                        if msg.command == "QUIT" {
                            let reason = msg.params.first().cloned();
                            send_line(&out_tx, &format!(
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
                                send_line(&out_tx, &formatter::rpl_motdstart(nick));
                                for line in lines {
                                    send_line(&out_tx, &formatter::rpl_motd(nick, line));
                                }
                                send_line(&out_tx, &formatter::rpl_endofmotd(nick));
                            } else {
                                send_line(&out_tx, &formatter::err_nomotd(nick));
                            }
                            continue;
                        }

                        // Async commands — need DB lookups or engine async methods
                        if matches!(msg.command.as_str(), "KICK" | "AWAY" | "INVITE" | "WHOIS") {
                            let replies = match msg.command.as_str() {
                                "KICK" => handle_kick(&engine, &db, *session_id, nick, &msg).await,
                                "AWAY" => handle_away(&engine, *session_id, nick, &msg).await,
                                "INVITE" => handle_invite(&engine, &db, *session_id, nick, &msg).await,
                                "WHOIS" => handle_whois(&engine, &db, nick, &msg).await,
                                _ => unreachable!(),
                            };
                            for reply in replies {
                                send_line(&out_tx, &reply);
                            }
                            continue;
                        }

                        let replies = commands::handle_command(&engine, *session_id, nick, &msg);
                        for reply in replies {
                            send_line(&out_tx, &reply);
                        }
                    }
                }
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    if let RegState::Registered { ref nick, .. } = state {
                        let lines = event_to_irc_lines(&engine, nick, &event, &caps);
                        for line in lines {
                            send_line(&out_tx, &line);
                        }
                    }
                }
            }
        } else {
            // Not registered yet — just read lines (with timeout)
            match tokio::time::timeout(IDLE_TIMEOUT, read_bounded_line(&mut reader, &mut line_buf))
                .await
            {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break, // EOF, error, or timeout
                Ok(Ok(_)) => {}
            }

            let line = line_buf.trim_end().to_string();
            line_buf.clear();

            if line.is_empty() {
                continue;
            }

            // Rate limit registration commands too (prevents brute-force token guessing)
            if !cmd_rate.check() {
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
                        send_line(&out_tx, &format!(":{sn} CAP * LS :{SUPPORTED_CAPS}"));
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
                                send_line(&out_tx, &format!(":{sn} CAP * ACK :{}", ack.join(" ")));
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
                        send_line(&out_tx, "AUTHENTICATE +");
                    } else if param == "*" {
                        // Client aborts SASL
                        send_line(
                            &out_tx,
                            &format!(":{sn} 906 * :SASL authentication aborted"),
                        );
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
                                match validate_irc_pass(&db, &passwd, nick_hint).await {
                                    Ok(Some(uid)) => {
                                        if let RegState::Unregistered { ref mut pass, .. } = state {
                                            *pass = Some(passwd.into_owned());
                                        }
                                        send_line(
                                            &out_tx,
                                            &format!(
                                                ":{sn} 900 * {uid} :You are now logged in as {uid}"
                                            ),
                                        );
                                        send_line(
                                            &out_tx,
                                            &format!(":{sn} 903 * :SASL authentication successful"),
                                        );
                                    }
                                    Ok(None) => {
                                        send_line(
                                            &out_tx,
                                            &format!(":{sn} 904 * :SASL authentication failed"),
                                        );
                                    }
                                    Err(_) => {
                                        send_line(
                                            &out_tx,
                                            &format!(":{sn} 904 * :SASL authentication failed"),
                                        );
                                    }
                                }
                            } else {
                                send_line(
                                    &out_tx,
                                    &format!(":{sn} 904 * :SASL authentication failed"),
                                );
                            }
                        } else {
                            send_line(&out_tx, &format!(":{sn} 904 * :SASL authentication failed"));
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
                        send_line(&out_tx, &formatter::err_nonicknamegiven("*"));
                        continue;
                    };

                    if !engine.is_nick_available(wanted_nick) {
                        send_line(&out_tx, &formatter::err_nicknameinuse("*", wanted_nick));
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
                    send_line(&out_tx, &formatter::err_notregistered());
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
                    match validate_irc_pass(&db, pass_token, nick_val).await {
                        Ok(Some(uid)) => Some(uid),
                        Ok(None) => {
                            send_line(
                                &out_tx,
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
                                &out_tx,
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
                    None
                };

                // Try to register with the engine
                match engine.connect(user_id, nick_val.clone(), Protocol::Irc, None) {
                    Ok((sid, rx)) => {
                        let nick_owned = nick_val.clone();

                        // Send welcome burst
                        send_line(&out_tx, &formatter::rpl_welcome(&nick_owned));
                        send_line(&out_tx, &formatter::rpl_yourhost(&nick_owned));
                        send_line(&out_tx, &formatter::rpl_created(&nick_owned));
                        send_line(&out_tx, &formatter::rpl_myinfo(&nick_owned));

                        // Send MOTD or ERR_NOMOTD
                        let motd = MOTD_LINES.get();
                        if let Some(lines) = motd
                            && !lines.is_empty()
                        {
                            send_line(&out_tx, &formatter::rpl_motdstart(&nick_owned));
                            for line in lines {
                                send_line(&out_tx, &formatter::rpl_motd(&nick_owned, line));
                            }
                            send_line(&out_tx, &formatter::rpl_endofmotd(&nick_owned));
                        } else {
                            send_line(&out_tx, &formatter::err_nomotd(&nick_owned));
                        }

                        state = RegState::Registered {
                            session_id: sid,
                            nick: nick_owned,
                        };
                        event_rx = Some(rx);
                    }
                    Err(e) => {
                        warn!(error = %e, "IRC registration failed");
                        send_line(&out_tx, &formatter::err_nicknameinuse("*", nick_val));
                    }
                }
            }
        }
    }

    // Disconnect from engine if registered
    if let RegState::Registered { session_id, nick } = state {
        engine.disconnect(session_id);
        info!(%peer, %nick, "IRC client disconnected");
    } else {
        info!(%peer, "IRC client disconnected (unregistered)");
    }

    write_handle.abort();
}

/// Validate an IRC PASS token against stored hashes.
/// Returns Ok(Some(user_id)) if the token matches, Ok(None) if not.
async fn validate_irc_pass(
    db: &SqlitePool,
    token: &str,
    nickname: &str,
) -> Result<Option<String>, String> {
    // Scoped lookup: only fetch tokens for this nickname (O(1) per user instead of O(n) global)
    let hashes = users::get_irc_token_hashes_by_nick(db, nickname)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    for (user_id, token_hash) in &hashes {
        if verify_irc_token(token, token_hash) {
            // Update last_used timestamp (fire-and-forget)
            let pool = db.clone();
            let uid = user_id.clone();
            let hash = token_hash.clone();
            tokio::spawn(async move {
                let _ = users::touch_irc_token(&pool, &uid, &hash).await;
            });
            return Ok(Some(user_id.clone()));
        }
    }

    Ok(None)
}

/// Handle IRC KICK command: KICK #channel user [:reason]
/// Requires async because it does a DB lookup (nickname → user_id) and calls engine.kick_member().
async fn handle_kick(
    engine: &ChatEngine,
    db: &SqlitePool,
    session_id: SessionId,
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

    let (server_id, _channel_name) = commands::parse_irc_channel(engine, target_channel);

    // Resolve target nickname → user_id via DB
    let target_user_id = match users::get_user_by_nickname(db, target_nick).await {
        Ok(Some((uid, ..))) => uid,
        Ok(None) => return vec![formatter::err_nosuchnick(nick, target_nick)],
        Err(e) => {
            warn!(error = %e, "KICK: DB error resolving nickname");
            return vec![formatter::err_nosuchnick(nick, target_nick)];
        }
    };

    match engine
        .kick_member(session_id, &server_id, &target_user_id, reason)
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
    session_id: SessionId,
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
    db: &SqlitePool,
    _session_id: SessionId,
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

    let (server_id, channel_name) = commands::parse_irc_channel(engine, target_channel);

    // Resolve target nickname → session_id
    let target_sid = match engine.get_session_id_by_nick(target_nick) {
        Some(sid) => sid,
        None => return vec![formatter::err_nosuchnick(nick, target_nick)],
    };

    // Look up target user_id
    let _target_user_id = match users::get_user_by_nickname(db, target_nick).await {
        Ok(Some((uid, ..))) => uid,
        _ => return vec![formatter::err_nosuchnick(nick, target_nick)],
    };

    // Join target to the channel
    if let Err(e) = engine.join_channel(target_sid, &server_id, &channel_name) {
        return vec![format!(":{sn} NOTICE {nick} :INVITE failed: {e}")];
    }

    let irc_channel = commands::to_irc_channel(engine, &server_id, &channel_name);
    vec![format!(":{sn} 341 {nick} {target_nick} {irc_channel}")]
}

/// Handle IRC WHOIS command with channel list and away status.
async fn handle_whois(
    engine: &ChatEngine,
    db: &SqlitePool,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let Some(target) = msg.params.first().or(msg.params.get(1)) else {
        return vec![formatter::err_needmoreparams(nick, "WHOIS")];
    };
    // Strip leading server param: WHOIS server target → use target
    let target = target.as_str();

    let Some(target_sid) = engine.get_session_id_by_nick(target) else {
        return vec![formatter::err_nosuchnick(nick, target)];
    };

    let mut lines = vec![
        formatter::rpl_whoisuser(nick, target),
        formatter::rpl_whoisserver(nick, target),
    ];

    // 319 RPL_WHOISCHANNELS — list channels the target is in
    let channels = engine.get_session_channels(target_sid);
    if !channels.is_empty() {
        let irc_names: Vec<String> = channels
            .iter()
            .map(|(sid, cname)| to_irc_channel(engine, sid, cname))
            .collect();
        lines.push(formatter::rpl_whoischannels(
            nick,
            target,
            &irc_names.join(" "),
        ));
    }

    // 301 RPL_AWAY — if the target has an away/idle status with a custom message
    if let Some(session) = engine.get_session(target_sid)
        && let Some(ref uid) = session.user_id
        && let Ok(Some(pres)) = presence::get_presence(db, uid).await
        && (pres.status == "idle" || pres.status == "dnd")
    {
        let away_msg = pres.custom_status.as_deref().unwrap_or("Away");
        lines.push(formatter::rpl_away(nick, target, away_msg));
    }

    lines.push(formatter::rpl_endofwhois(nick, target));
    lines
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
            tags.push(format!("msgid={id}"));
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
        | ChatEvent::CategoryList { .. }
        | ChatEvent::CategoryUpdate { .. }
        | ChatEvent::CategoryDelete { .. }
        | ChatEvent::ChannelReorder { .. }
        | ChatEvent::PresenceUpdate { .. }
        | ChatEvent::PresenceList { .. }
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
        | ChatEvent::TemplateList { .. }
        | ChatEvent::TemplateUpdate { .. }
        | ChatEvent::TemplateDelete { .. }
        // Phase 8: Integrations (web-only)
        | ChatEvent::WebhookList { .. }
        | ChatEvent::WebhookUpdate { .. }
        | ChatEvent::WebhookDelete { .. }
        | ChatEvent::SlashCommandList { .. }
        | ChatEvent::SlashCommandUpdate { .. }
        | ChatEvent::SlashCommandDelete { .. }
        | ChatEvent::InteractionCreate { .. }
        | ChatEvent::InteractionResponse { .. }
        | ChatEvent::BotTokenList { .. }
        | ChatEvent::OAuth2AppList { .. }
        | ChatEvent::OAuth2AppUpdate { .. }
        | ChatEvent::BlueskyProfileSync { .. }
        | ChatEvent::BlueskyShareResult { .. } => vec![],
    }
}

fn send_line(tx: &mpsc::UnboundedSender<String>, line: &str) {
    let _ = tx.send(line.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::events::{MemberInfo, PinnedMessageInfo, ThreadInfo};
    use chrono::Utc;
    use std::sync::Arc;
    use uuid::Uuid;

    /// Create a minimal ChatEngine with no database for unit tests.
    fn test_engine() -> Arc<ChatEngine> {
        Arc::new(ChatEngine::new(None))
    }

    /// Test helper — calls the inner (tag-free) event formatter.
    fn event_to_irc_lines(engine: &ChatEngine, my_nick: &str, event: &ChatEvent) -> Vec<String> {
        event_to_irc_lines_inner(engine, my_nick, event)
    }

    // ── Message event ──

    #[test]
    fn test_message_event_to_channel() {
        let engine = test_engine();
        let lines = event_to_irc_lines(
            &engine,
            "viewer",
            &ChatEvent::Message {
                id: Uuid::new_v4(),
                server_id: Some(DEFAULT_SERVER_ID.to_string()),
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
                id: Uuid::new_v4(),
                server_id: None,
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
                    },
                    MemberInfo {
                        nickname: "bob".into(),
                        avatar_url: None,
                        status: None,
                        custom_status: None,
                        status_emoji: None,
                        user_id: None,
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
                id: Uuid::new_v4(),
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
                id: Uuid::new_v4(),
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
                message_id: Uuid::new_v4(),
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
                message_id: Uuid::new_v4(),
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
                message_id: Uuid::new_v4(),
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
                    archived: false,
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
                    archived: true,
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
                    archived: false,
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
                roles: vec![],
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
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        send_line(&tx, "PRIVMSG #test :Hello");
        let received = rx.try_recv().unwrap();
        assert_eq!(received, "PRIVMSG #test :Hello");
    }

    #[test]
    fn test_send_line_closed_channel_does_not_panic() {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        drop(rx); // Close the receiver
        // Should not panic
        send_line(&tx, "PRIVMSG #test :Hello");
    }
}
