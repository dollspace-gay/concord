use super::{ChatEngine, ChatEvent, ClientCaps, DEFAULT_SERVER_ID, formatter, to_irc_channel};

pub(super) fn escape_ircv3_tag_value(value: &str) -> String {
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

pub(super) fn build_history_tag_prefix(
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
pub(super) fn build_tag_prefix(caps: &ClientCaps, event: &ChatEvent) -> String {
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
pub(super) fn event_to_irc_lines(
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
pub(super) fn event_to_irc_lines_inner(
    engine: &ChatEngine,
    my_nick: &str,
    event: &ChatEvent,
) -> Vec<String> {
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
