use super::{ChatEngine, ClientMessage};

pub(super) async fn dispatch(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    msg: ClientMessage,
) -> std::ops::ControlFlow<(), Result<(), String>> {
    match msg {
        ClientMessage::LifecycleCommand { .. } => unreachable!("lifecycle envelope was unwrapped"),
        ClientMessage::Sync {
            request_id,
            protocol_version,
            subscriptions,
            cursor,
            limit,
        } => {
            synchronization::sync(
                engine,
                session_id,
                (request_id, protocol_version, subscriptions, cursor, limit),
            )
            .await
        }
        ClientMessage::SendMessage {
            operation_generation,
            request_id,
            client_message_id,
            conversation_id,
            server_id,
            channel,
            content,
            content_format,
            reply_to,
            attachment_ids,
            mentions,
            nonce,
        } => {
            messaging::send_message(
                engine,
                session_id,
                messaging::SendMessage {
                    operation_generation,
                    request_id,
                    client_message_id,
                    conversation_id,
                    server_id,
                    channel,
                    content,
                    content_format,
                    reply_to,
                    attachment_ids,
                    mentions,
                    nonce,
                },
            )
            .await
        }
        ClientMessage::SendDirectMessage {
            operation_generation,
            request_id,
            client_message_id,
            recipient,
            content,
            content_format,
            reply_to,
            attachment_ids,
            nonce,
        } => {
            messaging::send_direct_message(
                engine,
                session_id,
                messaging::SendDirectMessage {
                    operation_generation,
                    request_id,
                    client_message_id,
                    recipient,
                    content,
                    content_format,
                    reply_to,
                    attachment_ids,
                    nonce,
                },
            )
            .await
        }
        ClientMessage::ListDirectConversations => {
            messaging::list_direct_conversations(engine, session_id).await
        }
        ClientMessage::JoinChannel { server_id, channel } => {
            channels::join_channel(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::PartChannel {
            server_id,
            channel,
            reason,
        } => channels::part_channel(engine, session_id, (server_id, channel, reason)).await,
        ClientMessage::SetTopic {
            server_id,
            channel,
            topic,
        } => channels::set_topic(engine, session_id, (server_id, channel, topic)).await,
        ClientMessage::FetchHistory {
            server_id,
            channel,
            before,
            limit,
        } => channels::fetch_history(engine, session_id, (server_id, channel, before, limit)).await,
        ClientMessage::ListChannels { server_id } => {
            channels::list_channels(engine, session_id, (server_id,)).await
        }
        ClientMessage::GetMembers { server_id, channel } => {
            channels::get_members(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::ListServers => servers::list_servers(engine, session_id).await,
        ClientMessage::CreateServer { name, icon_url } => {
            servers::create_server(engine, session_id, (name, icon_url)).await
        }
        ClientMessage::JoinServer { server_id } => {
            servers::join_server(engine, session_id, (server_id,)).await
        }
        ClientMessage::LeaveServer { server_id } => {
            servers::leave_server(engine, session_id, (server_id,)).await
        }
        ClientMessage::CreateChannel {
            server_id,
            name,
            category_id,
            is_private,
            channel_type,
        } => {
            channel_lifecycle::create_channel(
                engine,
                session_id,
                (server_id, name, category_id, is_private, channel_type),
            )
            .await
        }
        ClientMessage::DeleteChannel { server_id, channel } => {
            channel_lifecycle::delete_channel(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::DeleteServer { server_id } => {
            server_management::delete_server(engine, session_id, (server_id,)).await
        }
        ClientMessage::UpdateServer {
            server_id,
            name,
            icon_url,
        } => {
            server_management::update_server(engine, session_id, (server_id, name, icon_url)).await
        }
        ClientMessage::UpdateMemberRole {
            server_id,
            user_id,
            role,
        } => {
            server_management::update_member_role(engine, session_id, (server_id, user_id, role))
                .await
        }
        ClientMessage::EditMessage {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
            content,
            content_format,
            mentions,
        } => {
            message_mutations::edit_message(
                engine,
                session_id,
                (
                    operation_generation,
                    request_id,
                    client_message_id,
                    message_id,
                    content,
                    content_format,
                    mentions,
                ),
            )
            .await
        }
        ClientMessage::DeleteMessage {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
        } => {
            message_mutations::delete_message(
                engine,
                session_id,
                (
                    operation_generation,
                    request_id,
                    client_message_id,
                    message_id,
                ),
            )
            .await
        }
        ClientMessage::AddReaction {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
            emoji,
        } => {
            message_mutations::add_reaction(
                engine,
                session_id,
                (
                    operation_generation,
                    request_id,
                    client_message_id,
                    message_id,
                    emoji,
                ),
            )
            .await
        }
        ClientMessage::RemoveReaction {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
            emoji,
        } => {
            message_mutations::remove_reaction(
                engine,
                session_id,
                (
                    operation_generation,
                    request_id,
                    client_message_id,
                    message_id,
                    emoji,
                ),
            )
            .await
        }
        ClientMessage::Typing { server_id, channel } => {
            read_state::typing(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::MarkRead {
            operation_generation,
            request_id,
            client_message_id,
            conversation_id,
            server_id,
            channel,
            message_id,
        } => {
            read_state::mark_read(
                engine,
                session_id,
                (
                    operation_generation,
                    request_id,
                    client_message_id,
                    conversation_id,
                    server_id,
                    channel,
                    message_id,
                ),
            )
            .await
        }
        ClientMessage::GetUnreadCounts { server_id } => {
            read_state::get_unread_counts(engine, session_id, (server_id,)).await
        }
        ClientMessage::ListRoles { server_id } => {
            roles::list_roles(engine, session_id, (server_id,)).await
        }
        ClientMessage::CreateRole {
            server_id,
            name,
            color,
            permissions,
        } => roles::create_role(engine, session_id, (server_id, name, color, permissions)).await,
        ClientMessage::UpdateRole {
            server_id,
            role_id,
            name,
            color,
            permissions,
        } => {
            roles::update_role(
                engine,
                session_id,
                (server_id, role_id, name, color, permissions),
            )
            .await
        }
        ClientMessage::DeleteRole { server_id, role_id } => {
            roles::delete_role(engine, session_id, (server_id, role_id)).await
        }
        ClientMessage::AssignRole {
            server_id,
            user_id,
            role_id,
        } => roles::assign_role(engine, session_id, (server_id, user_id, role_id)).await,
        ClientMessage::RemoveRole {
            server_id,
            user_id,
            role_id,
        } => roles::remove_role(engine, session_id, (server_id, user_id, role_id)).await,
        ClientMessage::ListChannelPermissionOverrides {
            server_id,
            channel_id,
        } => {
            channel_permissions::list_channel_permission_overrides(
                engine,
                session_id,
                (server_id, channel_id),
            )
            .await
        }
        ClientMessage::SetChannelPermissionOverride {
            server_id,
            channel_id,
            target_type,
            target_id,
            allow_bits,
            deny_bits,
        } => {
            channel_permissions::set_channel_permission_override(
                engine,
                session_id,
                (
                    server_id,
                    channel_id,
                    target_type,
                    target_id,
                    allow_bits,
                    deny_bits,
                ),
            )
            .await
        }
        ClientMessage::DeleteChannelPermissionOverride {
            server_id,
            channel_id,
            target_type,
            target_id,
        } => {
            channel_permissions::delete_channel_permission_override(
                engine,
                session_id,
                (server_id, channel_id, target_type, target_id),
            )
            .await
        }
        ClientMessage::ListCategories { server_id } => {
            categories::list_categories(engine, session_id, (server_id,)).await
        }
        ClientMessage::CreateCategory { server_id, name } => {
            categories::create_category(engine, session_id, (server_id, name)).await
        }
        ClientMessage::UpdateCategory {
            server_id,
            category_id,
            name,
        } => categories::update_category(engine, session_id, (server_id, category_id, name)).await,
        ClientMessage::DeleteCategory {
            server_id,
            category_id,
        } => categories::delete_category(engine, session_id, (server_id, category_id)).await,
        ClientMessage::ReorderChannels {
            server_id,
            channels,
        } => categories::reorder_channels(engine, session_id, (server_id, channels)).await,
        ClientMessage::SetPresence {
            status,
            custom_status,
            status_emoji,
        } => {
            presence::set_presence(engine, session_id, (status, custom_status, status_emoji)).await
        }
        ClientMessage::GetPresences { server_id } => {
            presence::get_presences(engine, session_id, (server_id,)).await
        }
        ClientMessage::SetServerNickname {
            server_id,
            nickname,
        } => presence::set_server_nickname(engine, session_id, (server_id, nickname)).await,
        ClientMessage::SearchMessages {
            request_id,
            server_id,
            query,
            channel,
            limit,
            offset,
            continuation,
        } => {
            search::search_messages(
                engine,
                session_id,
                search::SearchMessages {
                    request_id,
                    server_id,
                    query,
                    channel,
                    limit,
                    offset,
                    continuation,
                },
            )
            .await
        }
        ClientMessage::UpdateNotificationSettings {
            server_id,
            channel_id,
            level,
            suppress_everyone,
            suppress_roles,
            muted,
            mute_until,
        } => {
            notifications::update_notification_settings(
                engine,
                session_id,
                notifications::UpdateNotificationSettings {
                    server_id,
                    channel_id,
                    level,
                    suppress_everyone,
                    suppress_roles,
                    muted,
                    mute_until,
                },
            )
            .await
        }
        ClientMessage::GetNotificationSettings { server_id } => {
            notifications::get_notification_settings(engine, session_id, (server_id,)).await
        }
        ClientMessage::GetUserProfile { user_id } => {
            profiles::get_user_profile(engine, session_id, (user_id,)).await
        }
        ClientMessage::PinMessage {
            server_id,
            channel,
            message_id,
        } => pins::pin_message(engine, session_id, (server_id, channel, message_id)).await,
        ClientMessage::UnpinMessage {
            server_id,
            channel,
            message_id,
        } => pins::unpin_message(engine, session_id, (server_id, channel, message_id)).await,
        ClientMessage::GetPinnedMessages { server_id, channel } => {
            pins::get_pinned_messages(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::CreateThread {
            server_id,
            parent_channel,
            name,
            message_id,
            is_private,
        } => {
            threads::create_thread(
                engine,
                session_id,
                (server_id, parent_channel, name, message_id, is_private),
            )
            .await
        }
        ClientMessage::ArchiveThread {
            server_id,
            thread_id,
        } => threads::archive_thread(engine, session_id, (server_id, thread_id)).await,
        ClientMessage::UnarchiveThread {
            server_id,
            thread_id,
        } => threads::unarchive_thread(engine, session_id, (server_id, thread_id)).await,
        ClientMessage::ListThreads { server_id, channel } => {
            threads::list_threads(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::CreateForumTag {
            server_id,
            channel,
            name,
            emoji,
            moderated,
        } => {
            forum_tags::create_forum_tag(
                engine,
                session_id,
                (server_id, channel, name, emoji, moderated),
            )
            .await
        }
        ClientMessage::UpdateForumTag {
            server_id,
            channel,
            tag_id,
            name,
            emoji,
            moderated,
            position,
        } => {
            forum_tags::update_forum_tag(
                engine,
                session_id,
                (server_id, channel, tag_id, name, emoji, moderated, position),
            )
            .await
        }
        ClientMessage::DeleteForumTag {
            server_id,
            channel,
            tag_id,
        } => forum_tags::delete_forum_tag(engine, session_id, (server_id, channel, tag_id)).await,
        ClientMessage::ListForumTags { server_id, channel } => {
            forum_tags::list_forum_tags(engine, session_id, (server_id, channel)).await
        }
        ClientMessage::SetThreadTags {
            server_id,
            thread_id,
            tag_ids,
        } => forum_tags::set_thread_tags(engine, session_id, (server_id, thread_id, tag_ids)).await,
        ClientMessage::GetThreadTags {
            server_id,
            thread_id,
        } => forum_tags::get_thread_tags(engine, session_id, (server_id, thread_id)).await,
        ClientMessage::AddBookmark { message_id, note } => {
            bookmarks::add_bookmark(engine, session_id, (message_id, note)).await
        }
        ClientMessage::RemoveBookmark { message_id } => {
            bookmarks::remove_bookmark(engine, session_id, (message_id,)).await
        }
        ClientMessage::ListBookmarks => bookmarks::list_bookmarks(engine, session_id).await,
        ClientMessage::KickMember {
            server_id,
            user_id,
            reason,
        } => moderation::kick_member(engine, session_id, (server_id, user_id, reason)).await,
        ClientMessage::BanMember {
            server_id,
            user_id,
            reason,
            delete_message_days,
        } => {
            moderation::ban_member(
                engine,
                session_id,
                (server_id, user_id, reason, delete_message_days),
            )
            .await
        }
        ClientMessage::UnbanMember { server_id, user_id } => {
            moderation::unban_member(engine, session_id, (server_id, user_id)).await
        }
        ClientMessage::ListBans { server_id } => {
            moderation::list_bans(engine, session_id, (server_id,)).await
        }
        ClientMessage::TimeoutMember {
            server_id,
            user_id,
            timeout_until,
            reason,
        } => {
            moderation::timeout_member(
                engine,
                session_id,
                (server_id, user_id, timeout_until, reason),
            )
            .await
        }
        ClientMessage::SetSlowMode {
            server_id,
            channel,
            seconds,
        } => moderation::set_slow_mode(engine, session_id, (server_id, channel, seconds)).await,
        ClientMessage::SetNsfw {
            server_id,
            channel,
            is_nsfw,
        } => moderation::set_nsfw(engine, session_id, (server_id, channel, is_nsfw)).await,
        ClientMessage::BulkDeleteMessages {
            server_id,
            channel,
            message_ids,
        } => {
            moderation::bulk_delete_messages(engine, session_id, (server_id, channel, message_ids))
                .await
        }
        ClientMessage::GetAuditLog {
            server_id,
            action_type,
            limit,
            before,
        } => {
            moderation::get_audit_log(engine, session_id, (server_id, action_type, limit, before))
                .await
        }
        ClientMessage::CreateAutomodRule {
            server_id,
            name,
            rule_type,
            config,
            action_type,
            timeout_duration_seconds,
        } => {
            automod::create_automod_rule(
                engine,
                session_id,
                (
                    server_id,
                    name,
                    rule_type,
                    config,
                    action_type,
                    timeout_duration_seconds,
                ),
            )
            .await
        }
        ClientMessage::UpdateAutomodRule {
            server_id,
            rule_id,
            name,
            enabled,
            config,
            action_type,
            timeout_duration_seconds,
        } => {
            automod::update_automod_rule(
                engine,
                session_id,
                (
                    server_id,
                    rule_id,
                    name,
                    enabled,
                    config,
                    action_type,
                    timeout_duration_seconds,
                ),
            )
            .await
        }
        ClientMessage::DeleteAutomodRule { server_id, rule_id } => {
            automod::delete_automod_rule(engine, session_id, (server_id, rule_id)).await
        }
        ClientMessage::ListAutomodRules { server_id } => {
            automod::list_automod_rules(engine, session_id, (server_id,)).await
        }
        ClientMessage::CreateInvite {
            server_id,
            max_uses,
            expires_at,
            channel_id,
        } => {
            invites::create_invite(
                engine,
                session_id,
                (server_id, max_uses, expires_at, channel_id),
            )
            .await
        }
        ClientMessage::ListInvites { server_id } => {
            invites::list_invites(engine, session_id, (server_id,)).await
        }
        ClientMessage::DeleteInvite {
            server_id,
            invite_id,
        } => invites::delete_invite(engine, session_id, (server_id, invite_id)).await,
        ClientMessage::UseInvite { code } => invites::use_invite(engine, session_id, (code,)).await,
        ClientMessage::CreateEvent {
            server_id,
            name,
            description,
            channel_id,
            start_time,
            end_time,
            image_url,
        } => {
            events::create_event(
                engine,
                session_id,
                events::CreateEvent {
                    server_id,
                    name,
                    description,
                    channel_id,
                    start_time,
                    end_time,
                    image_url,
                },
            )
            .await
        }
        ClientMessage::ListEvents { server_id } => {
            events::list_events(engine, session_id, (server_id,)).await
        }
        ClientMessage::UpdateEventStatus {
            server_id,
            event_id,
            status,
        } => events::update_event_status(engine, session_id, (server_id, event_id, status)).await,
        ClientMessage::DeleteEvent {
            server_id,
            event_id,
        } => events::delete_event(engine, session_id, (server_id, event_id)).await,
        ClientMessage::SetRsvp {
            server_id,
            event_id,
            status,
        } => events::set_rsvp(engine, session_id, (server_id, event_id, status)).await,
        ClientMessage::RemoveRsvp {
            server_id,
            event_id,
        } => events::remove_rsvp(engine, session_id, (server_id, event_id)).await,
        ClientMessage::ListRsvps { event_id } => {
            events::list_rsvps(engine, session_id, (event_id,)).await
        }
        ClientMessage::UpdateCommunitySettings {
            server_id,
            description,
            is_discoverable,
            welcome_message,
            rules_text,
            category,
        } => {
            community::update_community_settings(
                engine,
                session_id,
                (
                    server_id,
                    description,
                    is_discoverable,
                    welcome_message,
                    rules_text,
                    category,
                ),
            )
            .await
        }
        ClientMessage::GetCommunitySettings { server_id } => {
            community::get_community_settings(engine, session_id, (server_id,)).await
        }
        ClientMessage::DiscoverServers { category } => {
            community::discover_servers(engine, session_id, (category,)).await
        }
        ClientMessage::AcceptRules { server_id } => {
            community::accept_rules(engine, session_id, (server_id,)).await
        }
        ClientMessage::SetAnnouncementChannel {
            server_id,
            channel,
            is_announcement,
        } => {
            announcements::set_announcement_channel(
                engine,
                session_id,
                (server_id, channel, is_announcement),
            )
            .await
        }
        ClientMessage::FollowChannel {
            source_channel_id,
            target_channel_id,
        } => {
            announcements::follow_channel(
                engine,
                session_id,
                (source_channel_id, target_channel_id),
            )
            .await
        }
        ClientMessage::UnfollowChannel { follow_id } => {
            announcements::unfollow_channel(engine, session_id, (follow_id,)).await
        }
        ClientMessage::ListChannelFollows { channel_id } => {
            announcements::list_channel_follows(engine, session_id, (channel_id,)).await
        }
        ClientMessage::PublishAnnouncement { message_id } => {
            announcements::publish_announcement(engine, session_id, (message_id,)).await
        }
        ClientMessage::CreateTemplate {
            server_id,
            name,
            description,
        } => templates::create_template(engine, session_id, (server_id, name, description)).await,
        ClientMessage::ListTemplates { server_id } => {
            templates::list_templates(engine, session_id, (server_id,)).await
        }
        ClientMessage::DeleteTemplate {
            server_id,
            template_id,
        } => templates::delete_template(engine, session_id, (server_id, template_id)).await,
        ClientMessage::InstantiateTemplate {
            template_id,
            server_name,
        } => templates::instantiate_template(engine, session_id, (template_id, server_name)).await,
        ClientMessage::CreateWebhook {
            server_id,
            channel_id,
            name,
            webhook_type,
            url,
        } => {
            webhooks::create_webhook(
                engine,
                session_id,
                (server_id, channel_id, name, webhook_type, url),
            )
            .await
        }
        ClientMessage::ListWebhooks { server_id } => {
            webhooks::list_webhooks(engine, session_id, (server_id,)).await
        }
        ClientMessage::UpdateWebhook {
            webhook_id,
            name,
            avatar_url,
            channel_id,
        } => {
            webhooks::update_webhook(
                engine,
                session_id,
                (webhook_id, name, avatar_url, channel_id),
            )
            .await
        }
        ClientMessage::DeleteWebhook { webhook_id } => {
            webhooks::delete_webhook(engine, session_id, (webhook_id,)).await
        }
        ClientMessage::CreateBot {
            username,
            avatar_url,
        } => bots::create_bot(engine, session_id, (username, avatar_url)).await,
        ClientMessage::ListOwnedBots => bots::list_owned_bots(engine, session_id).await,
        ClientMessage::CreateBotToken {
            bot_user_id,
            name,
            scopes,
        } => bots::create_bot_token(engine, session_id, (bot_user_id, name, scopes)).await,
        ClientMessage::ListBotTokens { bot_user_id } => {
            bots::list_bot_tokens(engine, session_id, (bot_user_id,)).await
        }
        ClientMessage::DeleteBotToken { token_id } => {
            bots::delete_bot_token(engine, session_id, (token_id,)).await
        }
        ClientMessage::AddBotToServer {
            server_id,
            bot_user_id,
        } => bots::add_bot_to_server(engine, session_id, (server_id, bot_user_id)).await,
        ClientMessage::RemoveBotFromServer {
            server_id,
            bot_user_id,
        } => bots::remove_bot_from_server(engine, session_id, (server_id, bot_user_id)).await,
        ClientMessage::RegisterSlashCommand {
            server_id,
            name,
            description,
            options_json,
        } => {
            slash_commands::register_slash_command(
                engine,
                session_id,
                (server_id, name, description, options_json),
            )
            .await
        }
        ClientMessage::ListSlashCommands { server_id } => {
            slash_commands::list_slash_commands(engine, session_id, (server_id,)).await
        }
        ClientMessage::DeleteSlashCommand { command_id } => {
            slash_commands::delete_slash_command(engine, session_id, (command_id,)).await
        }
        ClientMessage::InvokeSlashCommand {
            request_id,
            server_id,
            channel,
            command_name,
            args_json,
        } => {
            interactions::invoke_slash_command(
                engine,
                session_id,
                (request_id, server_id, channel, command_name, args_json),
            )
            .await
        }
        ClientMessage::InvokeMessageComponent {
            request_id,
            message_id,
            custom_id,
            values,
        } => {
            interactions::invoke_message_component(
                engine,
                session_id,
                (request_id, message_id, custom_id, values),
            )
            .await
        }
        ClientMessage::RespondToInteraction {
            interaction_id,
            content,
            embeds_json,
            components_json,
            ephemeral,
        } => {
            interactions::respond_to_interaction(
                engine,
                session_id,
                (
                    interaction_id,
                    content,
                    embeds_json,
                    components_json,
                    ephemeral,
                ),
            )
            .await
        }
        ClientMessage::CreateOAuth2App {
            name,
            description,
            redirect_uris,
            client_type,
        } => {
            oauth_apps::create_o_auth2_app(
                engine,
                session_id,
                (name, description, redirect_uris, client_type),
            )
            .await
        }
        ClientMessage::ListOAuth2Apps => oauth_apps::list_o_auth2_apps(engine, session_id).await,
        ClientMessage::DeleteOAuth2App { app_id } => {
            oauth_apps::delete_o_auth2_app(engine, session_id, (app_id,)).await
        }
        ClientMessage::SetServerAvatar {
            server_id,
            avatar_url,
        } => server_profile::set_server_avatar(engine, session_id, (server_id, avatar_url)).await,
        ClientMessage::SetVanityCode {
            server_id,
            vanity_code,
        } => server_profile::set_vanity_code(engine, session_id, (server_id, vanity_code)).await,
        ClientMessage::GetServerLimits => {
            server_profile::get_server_limits(engine, session_id).await
        }
    }
}
use super::announcements;
use super::automod;
use super::bookmarks;
use super::bots;
use super::categories;
use super::channel_lifecycle;
use super::channel_permissions;
use super::channels;
use super::community;
use super::events;
use super::forum_tags;
use super::interactions;
use super::invites;
use super::message_mutations;
use super::messaging;
use super::moderation;
use super::notifications;
use super::oauth_apps;
use super::pins;
use super::presence;
use super::profiles;
use super::read_state;
use super::roles;
use super::search;
use super::server_management;
use super::server_profile;
use super::servers;
use super::slash_commands;
use super::synchronization;
use super::templates;
use super::threads;
use super::webhooks;
