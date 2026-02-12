use tracing::warn;

use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::SessionId;

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

    if let Some(server_name) = engine.get_server_name(server_id) {
        let bare_channel = channel_name.strip_prefix('#').unwrap_or(channel_name);
        format!("#{server_name}/{bare_channel}")
    } else {
        channel_name.to_string()
    }
}

/// Process a single IRC command from a registered (authenticated) client.
/// Returns a list of lines to send back to the client.
pub fn handle_command(
    engine: &ChatEngine,
    session_id: SessionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    match msg.command.as_str() {
        "JOIN" => handle_join(engine, session_id, nick, msg),
        "PART" => handle_part(engine, session_id, nick, msg),
        "PRIVMSG" => handle_privmsg(engine, session_id, nick, msg),
        "TOPIC" => handle_topic(engine, session_id, nick, msg),
        "NAMES" => handle_names(engine, nick, msg),
        "LIST" => handle_list(engine, nick, msg),
        "WHO" => handle_who(engine, nick, msg),
        "WHOIS" => handle_whois(engine, nick, msg),
        "QUIT" => vec![], // Handled at connection level
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
                    // Translate channel name for display
                    let (server_id, channel_name) = parse_irc_channel(engine, target);
                    let irc_channel = to_irc_channel(engine, &server_id, &channel_name);
                    vec![format!(
                        ":{} 324 {} {} +",
                        formatter::server_name(),
                        nick,
                        irc_channel
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

fn handle_join(
    engine: &ChatEngine,
    session_id: SessionId,
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

        let (server_id, channel_name) = parse_irc_channel(engine, channel);

        match engine.join_channel(session_id, &server_id, &channel_name) {
            Ok(()) => {}
            Err(e) => {
                warn!(error = %e, %channel, "JOIN failed");
                replies.push(formatter::err_nosuchchannel(nick, channel));
            }
        }
    }

    replies
}

fn handle_part(
    engine: &ChatEngine,
    session_id: SessionId,
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

        let (server_id, channel_name) = parse_irc_channel(engine, channel);

        if let Err(e) = engine.part_channel(session_id, &server_id, &channel_name, reason.clone()) {
            warn!(error = %e, %channel, "PART failed");
            replies.push(formatter::err_notonchannel(nick, channel));
        }
    }

    replies
}

fn handle_privmsg(
    engine: &ChatEngine,
    session_id: SessionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    if msg.params.len() < 2 {
        return vec![formatter::err_needmoreparams(nick, "PRIVMSG")];
    }

    let target = &msg.params[0];
    let content = &msg.params[1];

    if target.starts_with('#') {
        // Channel message — parse server/channel from IRC name
        let (server_id, channel_name) = parse_irc_channel(engine, target);
        if let Err(e) = engine.send_message(
            session_id,
            &server_id,
            &channel_name,
            content,
            None,
            None,
            None,
        ) {
            warn!(error = %e, %target, "PRIVMSG failed");
            return vec![formatter::err_nosuchnick(nick, target)];
        }
    } else {
        // DM — use default server
        if let Err(e) = engine.send_message(
            session_id,
            DEFAULT_SERVER_ID,
            target,
            content,
            None,
            None,
            None,
        ) {
            warn!(error = %e, %target, "PRIVMSG failed");
            return vec![formatter::err_nosuchnick(nick, target)];
        }
    }

    vec![]
}

fn handle_topic(
    engine: &ChatEngine,
    session_id: SessionId,
    nick: &str,
    msg: &IrcMessage,
) -> Vec<String> {
    let Some(channel_param) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "TOPIC")];
    };

    let (server_id, channel_name) = parse_irc_channel(engine, channel_param);
    let irc_channel = to_irc_channel(engine, &server_id, &channel_name);

    if let Some(new_topic) = msg.params.get(1) {
        if let Err(e) = engine.set_topic(session_id, &server_id, &channel_name, new_topic.clone()) {
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

fn handle_names(engine: &ChatEngine, nick: &str, msg: &IrcMessage) -> Vec<String> {
    let Some(channel_param) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "NAMES")];
    };

    let (server_id, channel_name) = parse_irc_channel(engine, channel_param);
    let irc_channel = to_irc_channel(engine, &server_id, &channel_name);

    match engine.get_members(&server_id, &channel_name) {
        Ok(member_infos) => {
            let nicks: Vec<String> = member_infos.iter().map(|m| m.nickname.clone()).collect();
            vec![
                formatter::rpl_namreply(nick, &irc_channel, &nicks),
                formatter::rpl_endofnames(nick, &irc_channel),
            ]
        }
        Err(_) => vec![formatter::rpl_endofnames(nick, &irc_channel)],
    }
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

fn handle_who(engine: &ChatEngine, nick: &str, msg: &IrcMessage) -> Vec<String> {
    let Some(target) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "WHO")];
    };

    let mut replies = Vec::new();

    if target.starts_with('#') {
        let (server_id, channel_name) = parse_irc_channel(engine, target);
        let irc_channel = to_irc_channel(engine, &server_id, &channel_name);

        if let Ok(members) = engine.get_members(&server_id, &channel_name) {
            for member in &members {
                replies.push(format!(
                    ":{} {} {} {} {} {} {} {} H :0 {}",
                    formatter::server_name(),
                    super::numerics::RPL_WHOREPLY,
                    nick,
                    irc_channel,
                    member.nickname,
                    formatter::server_name(),
                    formatter::server_name(),
                    member.nickname,
                    member.nickname,
                ));
            }
        }

        replies.push(format!(
            ":{} {} {} {} :End of /WHO list",
            formatter::server_name(),
            super::numerics::RPL_ENDOFWHO,
            nick,
            irc_channel,
        ));
    } else {
        replies.push(format!(
            ":{} {} {} {} :End of /WHO list",
            formatter::server_name(),
            super::numerics::RPL_ENDOFWHO,
            nick,
            target,
        ));
    }

    replies
}

fn handle_whois(engine: &ChatEngine, nick: &str, msg: &IrcMessage) -> Vec<String> {
    let Some(target) = msg.params.first() else {
        return vec![formatter::err_needmoreparams(nick, "WHOIS")];
    };

    if !engine.is_nick_available(target) {
        vec![
            formatter::rpl_whoisuser(nick, target),
            formatter::rpl_whoisserver(nick, target),
            formatter::rpl_endofwhois(nick, target),
        ]
    } else {
        vec![formatter::err_nosuchnick(nick, target)]
    }
}
