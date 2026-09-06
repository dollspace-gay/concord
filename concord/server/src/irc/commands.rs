use tracing::warn;

use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::ConnectionId;

use super::formatter;
use super::parser::IrcMessage;

/// Parse an IRC channel name into (server_id, engine_channel_name).
///
/// Format:
///   `#general`            -> (DEFAULT_SERVER_ID, "#general")   — default server
///   `#my-guild/general`   -> (server_id,         "#general")   — named server
///
/// If the server name doesn't match any known server, falls back to treating
/// the whole thing as a default-server channel name.
pub fn parse_irc_channel(engine: &ChatEngine, irc_name: &str) -> (String, String) {
    let bare = irc_name.strip_prefix('#').unwrap_or(irc_name);

    if let Some(slash_pos) = bare.find('/') {
        let server_name = &bare[..slash_pos];
        let channel_name = &bare[slash_pos + 1..];
        if let Some(server_id) = engine.find_server_by_name(server_name) {
            return (server_id, format!("#{channel_name}"));
        }
    }

    // Default: treat as default server channel
    (DEFAULT_SERVER_ID.to_string(), format!("#{bare}"))
}

/// Convert an engine (server_id, channel_name) back to an IRC channel name.
///
/// Default server channels keep their plain name (`#general`).
/// Non-default server channels become `#server-name/channel-name`.
pub fn to_irc_channel(engine: &ChatEngine, server_id: &str, channel_name: &str) -> String {
    if server_id == DEFAULT_SERVER_ID {
        return channel_name.to_string();
    }

    if let Some(server_name) = engine.get_server_alias(server_id) {
        let bare_channel = channel_name.strip_prefix('#').unwrap_or(channel_name);
        format!("#{server_name}/{bare_channel}")
    } else {
        channel_name.to_string()
    }
}

/// Process a single IRC command from a registered (authenticated) client.
/// Returns a list of lines to send back to the client.
pub async fn handle_command(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    match msg.command.as_str() {
        "JOIN" => handle_join(engine, session_id, nick, msg).await,
        "PART" => handle_part(engine, session_id, nick, msg).await,
        "PRIVMSG" => handle_privmsg(engine, session_id, nick, msg).await,
        "TOPIC" => handle_topic(engine, session_id, nick, msg).await,
        "NAMES" => vec![], // Handled async in connection.rs
        "LIST" => handle_list(engine, nick, msg),
        "WHO" => vec![],   // Handled async in connection.rs
        "WHOIS" => vec![], // Handled async in connection.rs
        "QUIT" => vec![],  // Handled at connection level
        "PING" => {
            let token = msg.params.first().map(|s| s.as_str()).unwrap_or("concord");
            vec![formatter::pong(token)]
        }
        "PONG" => vec![], // Just acknowledge, no response needed
        "NICK" | "USER" | "PASS" => {
            vec![formatter::err_alreadyregistered(nick)]
        }
        // CAP, MODE — common client sends these, just ignore or give minimal response
        "CAP" => {
            if msg.params.first().map(|s| s.as_str()) == Some("LS") {
                vec![format!(":{} CAP * LS :", formatter::server_name())]
            } else {
                vec![]
            }
        }
        "MODE" => {
            if let Some(target) = msg.params.first() {
                if target.starts_with('#') {
                    let Ok((server_id, channel_name)) =
                        resolve_actor_channel(engine, session_id, target).await
                    else {
                        return vec![formatter::err_nosuchchannel(nick, target)];
                    };
                    let irc_channel = to_irc_channel(engine, &server_id, &channel_name);
                    let modes = engine.get_channel_modes(&server_id, &channel_name);
                    vec![format!(
                        ":{} 324 {} {} {}",
                        formatter::server_name(),
                        nick,
                        irc_channel,
                        modes
                    )]
                } else {
                    vec![format!(":{} 221 {} +", formatter::server_name(), nick)]
                }
            } else {
                vec![formatter::err_needmoreparams(nick, "MODE")]
            }
        }
        "USERHOST" | "ISON" => {
            vec![]
        }
        _ => {
            warn!(command = %msg.command, "unknown IRC command");
            vec![formatter::err_unknowncommand(nick, &msg.command)]
        }
    }
}

async fn handle_join(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let Some(channels_param) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "JOIN")];
    };

    let mut replies = Vec::new();

    for channel in channels_param.split(',') {
        let channel = channel.trim();
        if channel.is_empty() {
            continue;
        }

        let Ok((server_id, channel_name)) =
            resolve_actor_channel(engine, session_id, channel).await
        else {
            replies.push(formatter::err_nosuchchannel(nick, channel));
            continue;
        };

        match engine
            .join_channel(session_id, &server_id, &channel_name)
            .await
        {
            Ok(()) => {}
            Err(e) => {
                warn!(error = %e, %channel, "JOIN failed");
                replies.push(formatter::err_nosuchchannel(nick, channel));
            }
        }
    }

    replies
}

async fn handle_part(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let Some(channels_param) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "PART")];
    };

    let reason = msg.params.get(1).cloned();
    let mut replies = Vec::new();

    for channel in channels_param.split(',') {
        let channel = channel.trim();
        if channel.is_empty() {
            continue;
        }

        let Ok((server_id, channel_name)) =
            resolve_actor_channel(engine, session_id, channel).await
        else {
            replies.push(formatter::err_notonchannel(nick, channel));
            continue;
        };

        if let Err(e) = engine.part_channel(session_id, &server_id, &channel_name, reason.clone()) {
            warn!(error = %e, %channel, "PART failed");
            replies.push(formatter::err_notonchannel(nick, channel));
        }
    }

    replies
}

async fn handle_privmsg(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    if msg.params.len() < 2 {
        return vec![formatter::err_needmoreparams(nick, "PRIVMSG")];
    }

    let target = &msg.params[0];
    let raw_content = &msg.params[1];

    // Handle CTCP messages (\x01...\x01)
    if let Some(ctcp) = parse_ctcp(raw_content) {
        return handle_ctcp(engine, session_id, nick, target, &ctcp).await;
    }

    if target.starts_with('#') {
        // Channel message — parse server/channel from IRC name
        let Ok((server_id, channel_name)) = resolve_actor_channel(engine, session_id, target).await
        else {
            return vec![formatter::err_nosuchnick(nick, target)];
        };
        let operation_id = uuid::Uuid::new_v4().to_string();
        if let Err(e) = engine
            .submit_channel_message(
                session_id,
                crate::engine::messaging::SendMessageCommand {
                    request_id: &operation_id,
                    client_message_id: &operation_id,
                    operation_generation: None,
                    conversation_id: None,
                    server_id: &server_id,
                    channel: &channel_name,
                    content: raw_content,
                    content_format: crate::engine::messaging::ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                    mentions: &[],
                },
                None,
            )
            .await
        {
            warn!(error = %e, %target, "PRIVMSG failed");
            return vec![formatter::err_nosuchnick(nick, target)];
        }
    } else {
        let operation_id = uuid::Uuid::new_v4().to_string();
        if let Err(error) = engine
            .submit_direct_message(
                session_id,
                crate::engine::messaging::SendDirectMessageCommand {
                    request_id: &operation_id,
                    client_message_id: &operation_id,
                    operation_generation: None,
                    recipient: target,
                    content: raw_content,
                    content_format: crate::engine::messaging::ContentFormat::Plain,
                    reply_to_id: None,
                    attachment_ids: &[],
                },
                None,
            )
            .await
        {
            warn!(%error, %target, "direct PRIVMSG failed");
            return vec![formatter::err_nosuchnick(nick, target)];
        }
    }

    vec![]
}

/// CTCP message content (between \x01 markers).
struct CtcpMessage {
    command: String,
    params: Option<String>,
}

/// Parse a CTCP message: \x01COMMAND [params]\x01
fn parse_ctcp(content: &str) -> Option<CtcpMessage> {
    let inner = content.strip_prefix('\x01')?.strip_suffix('\x01')?;
    if inner.is_empty() {
        return None;
    }
    let (command, params) = match inner.find(' ') {
        Some(pos) => (&inner[..pos], Some(inner[pos + 1..].to_string())),
        None => (inner, None),
    };
    Some(CtcpMessage {
        command: command.to_uppercase(),
        params,
    })
}

/// Handle CTCP commands: ACTION → /me, VERSION/PING/TIME → reply.
async fn handle_ctcp(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    target: &str,
    ctcp: &CtcpMessage,
) -> Vec<String> {
    match ctcp.command.as_str() {
        "ACTION" => {
            // Convert to /me format for the engine
            let action_text = ctcp.params.as_deref().unwrap_or("");
            let content = format!("/me {action_text}");
            if target.starts_with('#') {
                let Ok((server_id, channel_name)) =
                    resolve_actor_channel(engine, session_id, target).await
                else {
                    return vec![formatter::err_nosuchnick(nick, target)];
                };
                let operation_id = uuid::Uuid::new_v4().to_string();
                if let Err(e) = engine
                    .submit_channel_message(
                        session_id,
                        crate::engine::messaging::SendMessageCommand {
                            request_id: &operation_id,
                            client_message_id: &operation_id,
                            operation_generation: None,
                            conversation_id: None,
                            server_id: &server_id,
                            channel: &channel_name,
                            content: &content,
                            content_format: crate::engine::messaging::ContentFormat::Plain,
                            reply_to_id: None,
                            attachment_ids: &[],
                            mentions: &[],
                        },
                        None,
                    )
                    .await
                {
                    warn!(error = %e, %target, "CTCP ACTION failed");
                    return vec![formatter::err_nosuchnick(nick, target)];
                }
            } else {
                return vec![formatter::err_nosuchnick(nick, target)];
            }
            vec![]
        }
        "VERSION" => {
            vec![formatter::ctcp_reply(
                nick,
                "VERSION",
                "Concord IRC Bridge 0.1.0",
            )]
        }
        "PING" => {
            let token = ctcp.params.as_deref().unwrap_or("");
            vec![formatter::ctcp_reply(nick, "PING", token)]
        }
        "TIME" => {
            let now = chrono::Utc::now()
                .format("%a %b %d %H:%M:%S %Y UTC")
                .to_string();
            vec![formatter::ctcp_reply(nick, "TIME", &now)]
        }
        _ => vec![], // Unknown CTCP — silently ignore
    }
}

async fn handle_topic(
    engine: &ChatEngine,
    session_id: ConnectionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let Some(channel_param) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "TOPIC")];
    };

    let Ok((server_id, channel_name)) =
        resolve_actor_channel(engine, session_id, channel_param).await
    else {
        return vec![formatter::err_nosuchchannel(nick, channel_param)];
    };
    let irc_channel = to_irc_channel(engine, &server_id, &channel_name);

    if let Some(new_topic) = msg.params.get(1) {
        if let Err(e) = engine
            .set_topic(session_id, &server_id, &channel_name, new_topic.clone())
            .await
        {
            warn!(error = %e, %channel_name, "TOPIC set failed");
            return vec![formatter::err_notonchannel(nick, &irc_channel)];
        }
        vec![]
    } else {
        match engine.get_members(&server_id, &channel_name) {
            Ok(_) => {
                let channels = engine.list_channels(&server_id);
                if let Some(ch) = channels.iter().find(|c| c.name == channel_name) {
                    if ch.topic.is_empty() {
                        vec![formatter::rpl_notopic(nick, &irc_channel)]
                    } else {
                        vec![formatter::rpl_topic(nick, &irc_channel, &ch.topic)]
                    }
                } else {
                    vec![formatter::err_nosuchchannel(nick, &irc_channel)]
                }
            }
            Err(_) => vec![formatter::err_nosuchchannel(nick, &irc_channel)],
        }
    }
}

async fn resolve_actor_channel(
    engine: &ChatEngine,
    session_id: ConnectionId,
    irc_name: &str,
) -> Result<(String, String), String> {
    let actor = engine
        .get_authenticated_actor(session_id)
        .ok_or_else(|| "authentication unavailable".to_string())?;
    engine.resolve_irc_channel_for_actor(&actor, irc_name).await
}

fn handle_list(engine: &ChatEngine, nick: &str, msg: &IrcMessage) -> Vec<String> {
    // LIST with no args: show default server channels
    // LIST #server-name/* : show channels for a specific server
    let server_id = if let Some(pattern) = msg.params.first() {
        let bare = pattern.strip_prefix('#').unwrap_or(pattern);
        if let Some(server_name) = bare.strip_suffix("/*") {
            if let Some(sid) = engine.find_server_by_name(server_name) {
                sid
            } else {
                // Unknown server — return empty list
                return vec![formatter::rpl_listend(nick)];
            }
        } else {
            DEFAULT_SERVER_ID.to_string()
        }
    } else {
        DEFAULT_SERVER_ID.to_string()
    };

    let channels = engine.list_channels(&server_id);
    let mut replies = Vec::with_capacity(channels.len() + 1);

    for ch in &channels {
        let irc_name = to_irc_channel(engine, &server_id, &ch.name);
        replies.push(formatter::rpl_list(
            nick,
            &irc_name,
            ch.member_count,
            &ch.topic,
        ));
    }
    replies.push(formatter::rpl_listend(nick));

    replies
}
