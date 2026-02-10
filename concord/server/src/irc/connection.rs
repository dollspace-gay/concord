use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::auth::token::verify_irc_token;
use crate::db::queries::users;
use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::{ChatEvent, SessionId};
use crate::engine::user_session::Protocol;

use super::commands::{self, to_irc_channel};
use super::formatter;
use super::parser::IrcMessage;

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
pub async fn handle_irc_connection(stream: TcpStream, engine: Arc<ChatEngine>, db: SqlitePool) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".into());

    info!(%peer, "IRC client connected");

    let (reader, writer) = stream.into_split();
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
    let mut event_rx: Option<mpsc::UnboundedReceiver<ChatEvent>> = None;

    loop {
        // When registered, also select on engine events
        if let Some(ref mut rx) = event_rx {
            tokio::select! {
                result = reader.read_line(&mut line_buf) => {
                    match result {
                        Ok(0) | Err(_) => break, // Connection closed or error
                        Ok(_) => {}
                    }

                    let line = line_buf.trim_end().to_string();
                    line_buf.clear();

                    if line.is_empty() {
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

                        let replies = commands::handle_command(&engine, *session_id, nick, &msg);
                        for reply in replies {
                            send_line(&out_tx, &reply);
                        }
                    }
                }
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    if let RegState::Registered { ref nick, .. } = state {
                        let lines = event_to_irc_lines(&engine, nick, &event);
                        for line in lines {
                            send_line(&out_tx, &line);
                        }
                    }
                }
            }
        } else {
            // Not registered yet — just read lines
            match reader.read_line(&mut line_buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }

            let line = line_buf.trim_end().to_string();
            line_buf.clear();

            if line.is_empty() {
                continue;
            }

            let msg = match IrcMessage::parse(&line) {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Handle CAP during registration
            if msg.command == "CAP" {
                if msg.params.first().map(|s| s.as_str()) == Some("LS") {
                    send_line(
                        &out_tx,
                        &format!(":{} CAP * LS :", formatter::server_name()),
                    );
                }
                // CAP END just falls through
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
                        send_line(&out_tx, &formatter::err_nomotd(&nick_owned));

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
    let hashes = users::get_all_irc_token_hashes(db)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    for (user_id, stored_nick, token_hash) in &hashes {
        if stored_nick == nickname && verify_irc_token(token, token_hash) {
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

/// Convert a ChatEvent to IRC protocol lines for a specific recipient.
/// Uses the engine to translate (server_id, channel_name) to IRC format.
fn event_to_irc_lines(engine: &ChatEngine, my_nick: &str, event: &ChatEvent) -> Vec<String> {
    match event {
        ChatEvent::Message {
            server_id,
            from,
            target,
            content,
            ..
        } => {
            let irc_target = if target.starts_with('#') {
                let sid = server_id.as_deref().unwrap_or(DEFAULT_SERVER_ID);
                to_irc_channel(engine, sid, target)
            } else {
                target.clone()
            };
            vec![formatter::privmsg(from, &irc_target, content)]
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
            let nicks: Vec<String> = members.iter().map(|m| m.nickname.clone()).collect();
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
        // These events are WebSocket-specific and don't map to IRC
        ChatEvent::ChannelList { .. }
        | ChatEvent::History { .. }
        | ChatEvent::ServerList { .. } => vec![],
    }
}

fn send_line(tx: &mpsc::UnboundedSender<String>, line: &str) {
    let _ = tx.send(line.to_string());
}
