use super::{
    ChatEngine, ClientCaps, ConnectionId, GuardedCommandReplies, IrcMessage, Permissions,
    build_history_tag_prefix, formatter, resolve_registered_channel, to_irc_channel,
};

/// Determine the IRC prefix character (@, +, or none) for a user in a server.
/// @ = operator (MANAGE_CHANNELS, KICK_MEMBERS, BAN_MEMBERS, or ADMINISTRATOR)
/// + = voice (MANAGE_MESSAGES but not operator-level)
pub(super) async fn irc_prefix_for_user(
    engine: &ChatEngine,
    server_id: &str,
    user_id: &str,
) -> &'static str {
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
pub(super) async fn handle_names_async(
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

pub(super) async fn handle_list_async(
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

pub(super) async fn handle_history_async(
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
pub(super) async fn handle_who_async(
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
                    crate::irc::numerics::RPL_ENDOFWHO,
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
                crate::irc::numerics::RPL_WHOREPLY,
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
            crate::irc::numerics::RPL_ENDOFWHO,
            nick,
            irc_channel,
        ));
        return Ok((replies, DeliveryGuard::Stamps(vec![stamp])));
    } else {
        replies.push(format!(
            ":{} {} {} {} :End of /WHO list",
            formatter::server_name(),
            crate::irc::numerics::RPL_ENDOFWHO,
            nick,
            target,
        ));
    }

    Ok((replies, DeliveryGuard::ActorCurrent))
}
