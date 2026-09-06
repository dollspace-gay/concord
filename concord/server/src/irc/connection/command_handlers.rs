use super::{
    Actor, ChatEngine, ConnectionId, GuardedCommandReplies, IrcMessage, formatter, to_irc_channel,
};
use crate::irc::commands;

/// Handle IRC KICK command: KICK #channel user [:reason]
/// Requires async because it does a DB lookup (nickname → user_id) and calls engine.kick_member().
pub(super) async fn resolve_registered_channel(
    engine: &ChatEngine,
    session_id: ConnectionId,
    irc_name: &str,
) -> Result<(String, String), String> {
    let actor = engine
        .get_authenticated_actor(session_id)
        .ok_or_else(|| "authentication unavailable".to_string())?;
    engine.resolve_irc_channel_for_actor(&actor, irc_name).await
}

pub(super) async fn irc_command_delivery_guard(
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

pub(super) async fn handle_kick(
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
pub(super) async fn handle_away(
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
pub(super) async fn handle_invite(
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
pub(super) async fn handle_whois(
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
