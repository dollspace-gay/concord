use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::auth::token::validate_session_token;
use crate::db::queries::users;
use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::ChatEvent;
use crate::engine::permissions::Permissions;
use crate::engine::user_session::Protocol;

use super::app_state::AppState;

/// Client-to-server WebSocket message types.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    SendMessage {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        content: String,
        reply_to: Option<String>,
        attachment_ids: Option<Vec<String>>,
        nonce: Option<String>,
    },
    JoinChannel {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    PartChannel {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        reason: Option<String>,
    },
    SetTopic {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        topic: String,
    },
    FetchHistory {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        before: Option<String>,
        limit: Option<i64>,
    },
    ListChannels {
        #[serde(default = "default_server_id")]
        server_id: String,
    },
    GetMembers {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    ListServers,
    CreateServer {
        name: String,
        icon_url: Option<String>,
    },
    JoinServer {
        server_id: String,
    },
    LeaveServer {
        server_id: String,
    },
    CreateChannel {
        server_id: String,
        name: String,
        category_id: Option<String>,
        is_private: Option<bool>,
    },
    DeleteChannel {
        server_id: String,
        channel: String,
    },
    DeleteServer {
        server_id: String,
    },
    UpdateServer {
        server_id: String,
        name: Option<String>,
        icon_url: Option<String>,
    },
    UpdateMemberRole {
        server_id: String,
        user_id: String,
        role: String,
    },
    EditMessage {
        message_id: String,
        content: String,
    },
    DeleteMessage {
        message_id: String,
    },
    AddReaction {
        message_id: String,
        emoji: String,
    },
    RemoveReaction {
        message_id: String,
        emoji: String,
    },
    Typing {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    MarkRead {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        message_id: String,
    },
    GetUnreadCounts {
        #[serde(default = "default_server_id")]
        server_id: String,
    },
    // ── Roles ──
    ListRoles {
        server_id: String,
    },
    CreateRole {
        server_id: String,
        name: String,
        color: Option<String>,
        permissions: Option<i64>,
    },
    UpdateRole {
        server_id: String,
        role_id: String,
        name: String,
        color: Option<String>,
        permissions: i64,
    },
    DeleteRole {
        server_id: String,
        role_id: String,
    },
    AssignRole {
        server_id: String,
        user_id: String,
        role_id: String,
    },
    RemoveRole {
        server_id: String,
        user_id: String,
        role_id: String,
    },
    // ── Categories ──
    ListCategories {
        server_id: String,
    },
    CreateCategory {
        server_id: String,
        name: String,
    },
    UpdateCategory {
        server_id: String,
        category_id: String,
        name: String,
    },
    DeleteCategory {
        server_id: String,
        category_id: String,
    },
    // ── Channel organization ──
    ReorderChannels {
        server_id: String,
        channels: Vec<crate::engine::events::ChannelPositionInfo>,
    },
    // ── Phase 4: Presence ──
    SetPresence {
        status: String,
        custom_status: Option<String>,
        status_emoji: Option<String>,
    },
    GetPresences {
        server_id: String,
    },
    // ── Phase 4: Server Nicknames ──
    SetServerNickname {
        server_id: String,
        nickname: Option<String>,
    },
    // ── Phase 4: Search ──
    SearchMessages {
        server_id: String,
        query: String,
        channel: Option<String>,
        limit: Option<i64>,
        offset: Option<i64>,
    },
    // ── Phase 4: Notifications ──
    UpdateNotificationSettings {
        server_id: String,
        channel_id: Option<String>,
        level: String,
        suppress_everyone: Option<bool>,
        suppress_roles: Option<bool>,
        muted: Option<bool>,
        mute_until: Option<String>,
    },
    GetNotificationSettings {
        server_id: String,
    },
    // ── Phase 4: Profiles ──
    GetUserProfile {
        user_id: String,
    },
    // ── Phase 5: Pinning ──
    PinMessage {
        server_id: String,
        channel: String,
        message_id: String,
    },
    UnpinMessage {
        server_id: String,
        channel: String,
        message_id: String,
    },
    GetPinnedMessages {
        server_id: String,
        channel: String,
    },
    // ── Phase 5: Threads ──
    CreateThread {
        server_id: String,
        parent_channel: String,
        name: String,
        message_id: String,
        #[serde(default)]
        is_private: bool,
    },
    ArchiveThread {
        server_id: String,
        thread_id: String,
    },
    ListThreads {
        server_id: String,
        channel: String,
    },
    // ── Phase 5: Bookmarks ──
    AddBookmark {
        message_id: String,
        note: Option<String>,
    },
    RemoveBookmark {
        message_id: String,
    },
    ListBookmarks,
    // ── Phase 6: Moderation ──
    KickMember {
        server_id: String,
        user_id: String,
        reason: Option<String>,
    },
    BanMember {
        server_id: String,
        user_id: String,
        reason: Option<String>,
        #[serde(default)]
        delete_message_days: i32,
    },
    UnbanMember {
        server_id: String,
        user_id: String,
    },
    ListBans {
        server_id: String,
    },
    TimeoutMember {
        server_id: String,
        user_id: String,
        timeout_until: Option<String>,
        reason: Option<String>,
    },
    SetSlowMode {
        server_id: String,
        channel: String,
        seconds: i32,
    },
    SetNsfw {
        server_id: String,
        channel: String,
        is_nsfw: bool,
    },
    BulkDeleteMessages {
        server_id: String,
        channel: String,
        message_ids: Vec<String>,
    },
    GetAuditLog {
        server_id: String,
        action_type: Option<String>,
        limit: Option<i64>,
        before: Option<String>,
    },
    // ── Phase 6: AutoMod ──
    CreateAutomodRule {
        server_id: String,
        name: String,
        rule_type: String,
        config: String,
        action_type: String,
        timeout_duration_seconds: Option<i32>,
    },
    UpdateAutomodRule {
        server_id: String,
        rule_id: String,
        name: String,
        enabled: bool,
        config: String,
        action_type: String,
        timeout_duration_seconds: Option<i32>,
    },
    DeleteAutomodRule {
        server_id: String,
        rule_id: String,
    },
    ListAutomodRules {
        server_id: String,
    },
    // ── Phase 7: Community & Discovery ──
    CreateInvite {
        server_id: String,
        max_uses: Option<i32>,
        expires_at: Option<String>,
        channel_id: Option<String>,
    },
    ListInvites {
        server_id: String,
    },
    DeleteInvite {
        server_id: String,
        invite_id: String,
    },
    UseInvite {
        code: String,
    },
    CreateEvent {
        server_id: String,
        name: String,
        description: Option<String>,
        channel_id: Option<String>,
        start_time: String,
        end_time: Option<String>,
        image_url: Option<String>,
    },
    ListEvents {
        server_id: String,
    },
    UpdateEventStatus {
        server_id: String,
        event_id: String,
        status: String,
    },
    DeleteEvent {
        server_id: String,
        event_id: String,
    },
    SetRsvp {
        server_id: String,
        event_id: String,
        status: String,
    },
    RemoveRsvp {
        server_id: String,
        event_id: String,
    },
    ListRsvps {
        event_id: String,
    },
    UpdateCommunitySettings {
        server_id: String,
        description: Option<String>,
        is_discoverable: bool,
        welcome_message: Option<String>,
        rules_text: Option<String>,
        category: Option<String>,
    },
    GetCommunitySettings {
        server_id: String,
    },
    DiscoverServers {
        category: Option<String>,
    },
    AcceptRules {
        server_id: String,
    },
    SetAnnouncementChannel {
        server_id: String,
        channel: String,
        is_announcement: bool,
    },
    FollowChannel {
        source_channel_id: String,
        target_channel_id: String,
    },
    UnfollowChannel {
        follow_id: String,
    },
    ListChannelFollows {
        channel_id: String,
    },
    CreateTemplate {
        server_id: String,
        name: String,
        description: Option<String>,
    },
    ListTemplates {
        server_id: String,
    },
    DeleteTemplate {
        server_id: String,
        template_id: String,
    },
    // ── Phase 8: Integrations & Bots ──
    CreateWebhook {
        server_id: String,
        channel_id: String,
        name: String,
        webhook_type: String,
        url: Option<String>,
    },
    ListWebhooks {
        server_id: String,
    },
    UpdateWebhook {
        webhook_id: String,
        name: String,
        avatar_url: Option<String>,
        channel_id: String,
    },
    DeleteWebhook {
        webhook_id: String,
    },
    CreateBot {
        username: String,
        avatar_url: Option<String>,
    },
    CreateBotToken {
        bot_user_id: String,
        name: String,
        scopes: Option<String>,
    },
    ListBotTokens {
        bot_user_id: String,
    },
    DeleteBotToken {
        token_id: String,
    },
    AddBotToServer {
        server_id: String,
        bot_user_id: String,
    },
    RemoveBotFromServer {
        server_id: String,
        bot_user_id: String,
    },
    RegisterSlashCommand {
        server_id: String,
        name: String,
        description: String,
        options_json: Option<String>,
    },
    ListSlashCommands {
        server_id: String,
    },
    DeleteSlashCommand {
        command_id: String,
    },
    InvokeSlashCommand {
        server_id: String,
        channel: String,
        command_name: String,
        args_json: Option<String>,
    },
    RespondToInteraction {
        interaction_id: String,
        content: Option<String>,
        embeds_json: Option<String>,
        components_json: Option<String>,
        ephemeral: Option<bool>,
    },
    CreateOAuth2App {
        name: String,
        description: Option<String>,
        redirect_uris: Vec<String>,
    },
    ListOAuth2Apps,
    DeleteOAuth2App {
        app_id: String,
    },

    // ── Premium-for-Free features ──
    SetServerAvatar {
        server_id: String,
        avatar_url: Option<String>,
    },
    SetVanityCode {
        server_id: String,
        vanity_code: Option<String>,
    },
    GetServerLimits,
}

fn default_server_id() -> String {
    DEFAULT_SERVER_ID.to_string()
}

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Try cookie-based auth first
    let (nickname, user_id, avatar_url) = if let Some(cookie) = jar.get("concord_session") {
        if let Ok(claims) = validate_session_token(cookie.value(), &state.auth_config.jwt_secret) {
            match users::get_user(&state.db, &claims.sub).await {
                Ok(Some((id, username, _email, avatar))) => (username, Some(id), avatar),
                _ => {
                    return (axum::http::StatusCode::UNAUTHORIZED, "User not found")
                        .into_response();
                }
            }
        } else {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "Invalid session token",
            )
                .into_response();
        }
    } else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Not authenticated. Provide a valid session cookie.",
        )
            .into_response();
    };

    let engine = state.engine.clone();
    ws.max_message_size(64 * 1024) // 64 KB max WS message
        .on_upgrade(move |socket| {
            handle_ws_connection(socket, engine, user_id, nickname, avatar_url)
        })
        .into_response()
}

async fn handle_ws_connection(
    socket: WebSocket,
    engine: Arc<ChatEngine>,
    user_id: Option<String>,
    nickname: String,
    avatar_url: Option<String>,
) {
    let (session_id, mut event_rx) =
        match engine.connect(user_id, nickname.clone(), Protocol::WebSocket, avatar_url) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(%nickname, error = %e, "WebSocket connection rejected");
                return;
            }
        };

    let (mut ws_sender, mut ws_receiver) = socket.split();

    let write_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match serde_json::to_string(&event) {
                Ok(json) => {
                    if ws_sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!(error = %e, "failed to serialize event");
                }
            }
        }
    });

    let engine_ref = engine.clone();
    let mut ws_command_count: u32 = 0;
    let mut ws_command_window_start = Instant::now();
    const WS_COMMANDS_PER_SECOND: u32 = 30;

    loop {
        let msg = match tokio::time::timeout(std::time::Duration::from_secs(90), ws_receiver.next())
            .await
        {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => {
                warn!(error = %e, "WebSocket read error");
                break;
            }
            Ok(None) => break, // Stream closed
            Err(_) => {
                info!(%session_id, %nickname, "WebSocket idle timeout, disconnecting");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                ws_command_count += 1;
                if ws_command_window_start.elapsed() >= std::time::Duration::from_secs(1) {
                    ws_command_count = 1;
                    ws_command_window_start = Instant::now();
                } else if ws_command_count > WS_COMMANDS_PER_SECOND {
                    if let Some(session) = engine_ref.get_session(session_id) {
                        let _ = session.send(ChatEvent::Error {
                            code: "RATE_LIMITED".to_string(),
                            message: "Rate limited: too many commands per second".to_string(),
                        });
                    }
                    continue;
                }
                handle_client_message(&engine_ref, session_id, &text).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    engine.disconnect(session_id);
    write_handle.abort();
    info!(%session_id, %nickname, "WebSocket connection closed");
}

async fn handle_client_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::SessionId,
    text: &str,
) {
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "invalid client message");
            return;
        }
    };

    let result = match msg {
        ClientMessage::SendMessage {
            server_id,
            channel,
            content,
            reply_to,
            attachment_ids,
            nonce,
        } => engine.send_message(
            session_id,
            &server_id,
            &channel,
            &content,
            reply_to.as_deref(),
            attachment_ids.as_deref(),
            nonce.as_deref(),
        ),
        ClientMessage::JoinChannel { server_id, channel } => {
            engine.join_channel(session_id, &server_id, &channel)
        }
        ClientMessage::PartChannel {
            server_id,
            channel,
            reason,
        } => engine.part_channel(session_id, &server_id, &channel, reason),
        ClientMessage::SetTopic {
            server_id,
            channel,
            topic,
        } => engine.set_topic(session_id, &server_id, &channel, topic),
        ClientMessage::FetchHistory {
            server_id,
            channel,
            before,
            limit,
        } => {
            // Verify the user is a member of this server
            let is_member = engine
                .get_session(session_id)
                .and_then(|s| {
                    s.user_id
                        .as_ref()
                        .map(|uid| engine.user_is_server_member(&server_id, uid))
                })
                .unwrap_or(false);
            if !is_member {
                Err("You are not a member of this server".into())
            } else {
                let user_id = engine
                    .get_session(session_id)
                    .and_then(|s| s.user_id.clone());
                let limit = limit.unwrap_or(50).clamp(1, 200);
                match engine
                    .fetch_history(
                        &server_id,
                        &channel,
                        before.as_deref(),
                        limit,
                        user_id.as_deref(),
                    )
                    .await
                {
                    Ok((messages, has_more)) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::History {
                                server_id,
                                channel,
                                messages,
                                has_more,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        }
        ClientMessage::ListChannels { server_id } => {
            // Verify the user is a member of this server
            let is_member = engine
                .get_session(session_id)
                .and_then(|s| {
                    s.user_id
                        .as_ref()
                        .map(|uid| engine.user_is_server_member(&server_id, uid))
                })
                .unwrap_or(false);
            if !is_member {
                Err("You are not a member of this server".into())
            } else {
                let channels = engine.list_channels(&server_id);
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::ChannelList {
                        server_id,
                        channels,
                    });
                }
                Ok(())
            }
        }
        ClientMessage::GetMembers { server_id, channel } => {
            // Verify the user is a member of this server
            let is_member = engine
                .get_session(session_id)
                .and_then(|s| {
                    s.user_id
                        .as_ref()
                        .map(|uid| engine.user_is_server_member(&server_id, uid))
                })
                .unwrap_or(false);
            if !is_member {
                Err("You are not a member of this server".into())
            } else {
                match engine.get_members(&server_id, &channel) {
                    Ok(member_infos) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::Names {
                                server_id,
                                channel,
                                members: member_infos,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        }
        ClientMessage::ListServers => {
            if let Some(session) = engine.get_session(session_id) {
                let servers = if let Some(ref uid) = session.user_id {
                    engine.list_servers_for_user(uid).await
                } else {
                    vec![] // unauthenticated sessions see no servers
                };
                let _ = session.send(ChatEvent::ServerList { servers });
            }
            Ok(())
        }
        ClientMessage::CreateServer { name, icon_url } => {
            let session = engine.get_session(session_id);
            let user_id = session.as_ref().and_then(|s| s.user_id.clone());
            let Some(uid) = user_id else {
                return send_error(
                    engine,
                    session_id,
                    "AUTH_REQUIRED",
                    "Must be authenticated to create a server",
                );
            };
            match engine.create_server(name, uid, icon_url).await {
                Ok(_server_id) => {
                    if let Some(session) = engine.get_session(session_id)
                        && let Some(ref uid) = session.user_id
                    {
                        let servers = engine.list_servers_for_user(uid).await;
                        let _ = session.send(ChatEvent::ServerList { servers });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::JoinServer { server_id } => {
            let session = engine.get_session(session_id);
            let user_id = session.as_ref().and_then(|s| s.user_id.clone());
            let Some(uid) = user_id else {
                return send_error(
                    engine,
                    session_id,
                    "AUTH_REQUIRED",
                    "Must be authenticated to join a server",
                );
            };
            match engine.join_server(&uid, &server_id).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id)
                        && let Some(ref uid) = session.user_id
                    {
                        let servers = engine.list_servers_for_user(uid).await;
                        let _ = session.send(ChatEvent::ServerList { servers });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::LeaveServer { server_id } => {
            let session = engine.get_session(session_id);
            let user_id = session.as_ref().and_then(|s| s.user_id.clone());
            let Some(uid) = user_id else {
                return send_error(
                    engine,
                    session_id,
                    "AUTH_REQUIRED",
                    "Must be authenticated to leave a server",
                );
            };
            match engine.leave_server(&uid, &server_id).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id)
                        && let Some(ref uid) = session.user_id
                    {
                        let servers = engine.list_servers_for_user(uid).await;
                        let _ = session.send(ChatEvent::ServerList { servers });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::CreateChannel {
            server_id,
            name,
            category_id,
            is_private,
        } => {
            match engine
                .require_permission(session_id, &server_id, None, Permissions::MANAGE_CHANNELS)
                .await
            {
                Ok(_) => match engine
                    .create_channel_in_server(
                        &server_id,
                        &name,
                        category_id.as_deref(),
                        is_private.unwrap_or(false),
                    )
                    .await
                {
                    Ok(_) => {
                        let channels = engine.list_channels(&server_id);
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::ChannelList {
                                server_id,
                                channels,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteChannel { server_id, channel } => {
            match engine
                .require_permission(session_id, &server_id, None, Permissions::MANAGE_CHANNELS)
                .await
            {
                Ok(_) => match engine.delete_channel_in_server(&server_id, &channel).await {
                    Ok(()) => {
                        let channels = engine.list_channels(&server_id);
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::ChannelList {
                                server_id,
                                channels,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteServer { server_id } => {
            let session = engine.get_session(session_id);
            let user_id = session.as_ref().and_then(|s| s.user_id.clone());
            let Some(uid) = user_id else {
                return send_error(
                    engine,
                    session_id,
                    "AUTH_REQUIRED",
                    "Must be authenticated to delete a server",
                );
            };
            if !engine.is_server_owner(&server_id, &uid) {
                return send_error(
                    engine,
                    session_id,
                    "FORBIDDEN",
                    "Only the server owner can delete it",
                );
            }
            match engine.delete_server(&server_id).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id)
                        && let Some(ref uid) = session.user_id
                    {
                        let servers = engine.list_servers_for_user(uid).await;
                        let _ = session.send(ChatEvent::ServerList { servers });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::UpdateServer {
            server_id,
            name,
            icon_url,
        } => {
            match engine
                .require_permission(session_id, &server_id, None, Permissions::MANAGE_SERVER)
                .await
            {
                Ok(_) => {
                    match engine
                        .update_server_settings(&server_id, name.as_deref(), icon_url.as_deref())
                        .await
                    {
                        Ok(()) => {
                            // Send updated server list to the requester
                            if let Some(session) = engine.get_session(session_id)
                                && let Some(ref uid) = session.user_id
                            {
                                let servers = engine.list_servers_for_user(uid).await;
                                let _ = session.send(ChatEvent::ServerList { servers });
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::UpdateMemberRole {
            server_id,
            user_id,
            role,
        } => {
            // Require MANAGE_ROLES permission
            match engine
                .require_permission(session_id, &server_id, None, Permissions::MANAGE_ROLES)
                .await
            {
                Ok(_) => {
                    // Prevent assigning "owner" through this path
                    if role == "owner" {
                        Err("Cannot assign owner role — use ownership transfer".into())
                    } else if engine.is_server_owner(&server_id, &user_id) {
                        // Prevent modifying the server owner's role
                        Err("Cannot change the server owner's role".into())
                    } else {
                        // Role hierarchy: caller must outrank the target role
                        let mut hierarchy_ok = true;
                        if let Some(session) = engine.get_session(session_id)
                            && let Some(ref caller_uid) = session.user_id
                            && !engine.is_server_owner(&server_id, caller_uid)
                        {
                            let caller_role = engine.get_server_role(&server_id, caller_uid).await;
                            let target_role = match role.as_str() {
                                "admin" => Some(crate::engine::permissions::ServerRole::Admin),
                                "moderator" => {
                                    Some(crate::engine::permissions::ServerRole::Moderator)
                                }
                                "member" => Some(crate::engine::permissions::ServerRole::Member),
                                _ => None,
                            };
                            if let (Some(caller), Some(target)) = (caller_role, target_role)
                                && target >= caller
                            {
                                hierarchy_ok = false;
                            }
                        }
                        if !hierarchy_ok {
                            Err("Cannot assign a role at or above your own level".into())
                        } else if let Some(pool) = engine.db() {
                            crate::db::queries::servers::update_member_role(
                                pool, &server_id, &user_id, &role,
                            )
                            .await
                            .map_err(|e| format!("Failed to update role: {e}"))
                        } else {
                            Err("No database configured".into())
                        }
                    }
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::EditMessage {
            message_id,
            content,
        } => engine.edit_message(session_id, &message_id, &content).await,
        ClientMessage::DeleteMessage { message_id } => {
            engine.delete_message(session_id, &message_id).await
        }
        ClientMessage::AddReaction { message_id, emoji } => {
            engine.add_reaction(session_id, &message_id, &emoji).await
        }
        ClientMessage::RemoveReaction { message_id, emoji } => {
            engine
                .remove_reaction(session_id, &message_id, &emoji)
                .await
        }
        ClientMessage::Typing { server_id, channel } => {
            engine.send_typing(session_id, &server_id, &channel)
        }
        ClientMessage::MarkRead {
            server_id,
            channel,
            message_id,
        } => {
            engine
                .mark_read(session_id, &server_id, &channel, &message_id)
                .await
        }
        ClientMessage::GetUnreadCounts { server_id } => {
            match engine.get_unread_counts(session_id, &server_id).await {
                Ok(counts) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::UnreadCounts { server_id, counts });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        // ── Roles ──
        ClientMessage::ListRoles { server_id } => match engine.list_roles(&server_id).await {
            Ok(roles) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::RoleList { server_id, roles });
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        ClientMessage::CreateRole {
            server_id,
            name,
            color,
            permissions,
        } => {
            let perms = permissions.unwrap_or(0);
            // Prevent non-owners from setting ADMINISTRATOR bit
            let requested =
                crate::engine::permissions::Permissions::from_bits_truncate(perms as u64);
            if requested.contains(crate::engine::permissions::Permissions::ADMINISTRATOR)
                && !engine.is_server_owner(
                    &server_id,
                    &engine
                        .get_session(session_id)
                        .and_then(|s| s.user_id.clone())
                        .unwrap_or_default(),
                )
            {
                Err("Only the server owner can grant ADMINISTRATOR permission".into())
            } else {
                match engine
                    .require_permission(
                        session_id,
                        &server_id,
                        None,
                        crate::engine::permissions::Permissions::MANAGE_ROLES,
                    )
                    .await
                {
                    Ok(_) => match engine
                        .create_role(&server_id, &name, color.as_deref(), perms)
                        .await
                    {
                        Ok(role) => {
                            if let Some(session) = engine.get_session(session_id) {
                                let _ = session.send(ChatEvent::RoleUpdate { server_id, role });
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                }
            }
        }
        ClientMessage::UpdateRole {
            server_id,
            role_id,
            name,
            color,
            permissions,
        } => {
            // Prevent non-owners from setting ADMINISTRATOR bit
            let requested =
                crate::engine::permissions::Permissions::from_bits_truncate(permissions as u64);
            if requested.contains(crate::engine::permissions::Permissions::ADMINISTRATOR)
                && !engine.is_server_owner(
                    &server_id,
                    &engine
                        .get_session(session_id)
                        .and_then(|s| s.user_id.clone())
                        .unwrap_or_default(),
                )
            {
                Err("Only the server owner can grant ADMINISTRATOR permission".into())
            } else {
                match engine
                    .require_permission(
                        session_id,
                        &server_id,
                        None,
                        crate::engine::permissions::Permissions::MANAGE_ROLES,
                    )
                    .await
                {
                    Ok(actor_uid) => {
                        // Role hierarchy: can't edit roles at or above your own
                        match engine
                            .check_role_hierarchy(&server_id, &actor_uid, &role_id)
                            .await
                        {
                            Err(e) => Err(e),
                            Ok(()) => match engine
                                .update_role(
                                    &server_id,
                                    &role_id,
                                    &name,
                                    color.as_deref(),
                                    permissions,
                                )
                                .await
                            {
                                Ok(role) => {
                                    if let Some(session) = engine.get_session(session_id) {
                                        let _ =
                                            session.send(ChatEvent::RoleUpdate { server_id, role });
                                    }
                                    Ok(())
                                }
                                Err(e) => Err(e),
                            },
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        }
        ClientMessage::DeleteRole { server_id, role_id } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_ROLES,
                )
                .await
            {
                Ok(actor_uid) => {
                    // Role hierarchy: can't delete roles at or above your own
                    match engine
                        .check_role_hierarchy(&server_id, &actor_uid, &role_id)
                        .await
                    {
                        Err(e) => Err(e),
                        Ok(()) => match engine.delete_role(&server_id, &role_id).await {
                            Ok(()) => {
                                if let Some(session) = engine.get_session(session_id) {
                                    let _ =
                                        session.send(ChatEvent::RoleDelete { server_id, role_id });
                                }
                                Ok(())
                            }
                            Err(e) => Err(e),
                        },
                    }
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::AssignRole {
            server_id,
            user_id,
            role_id,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_ROLES,
                )
                .await
            {
                Ok(actor_uid) => match engine
                    .assign_role(&server_id, &actor_uid, &user_id, &role_id)
                    .await
                {
                    Ok(role_ids) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::MemberRoleUpdate {
                                server_id,
                                user_id,
                                role_ids,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        ClientMessage::RemoveRole {
            server_id,
            user_id,
            role_id,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_ROLES,
                )
                .await
            {
                Ok(actor_uid) => match engine
                    .remove_role(&server_id, &actor_uid, &user_id, &role_id)
                    .await
                {
                    Ok(role_ids) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::MemberRoleUpdate {
                                server_id,
                                user_id,
                                role_ids,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        // ── Categories ──
        ClientMessage::ListCategories { server_id } => {
            match engine.list_categories(&server_id).await {
                Ok(categories) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::CategoryList {
                            server_id,
                            categories,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::CreateCategory { server_id, name } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_CHANNELS,
                )
                .await
            {
                Ok(_) => match engine.create_category(&server_id, &name).await {
                    Ok(category) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::CategoryUpdate {
                                server_id,
                                category,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        ClientMessage::UpdateCategory {
            server_id,
            category_id,
            name,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_CHANNELS,
                )
                .await
            {
                Ok(_) => match engine.update_category(&category_id, &name).await {
                    Ok(category) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::CategoryUpdate {
                                server_id,
                                category,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteCategory {
            server_id,
            category_id,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_CHANNELS,
                )
                .await
            {
                Ok(_) => match engine.delete_category(&category_id).await {
                    Ok(()) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::CategoryDelete {
                                server_id,
                                category_id,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        // ── Channel organization ──
        ClientMessage::ReorderChannels {
            server_id,
            channels,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_CHANNELS,
                )
                .await
            {
                Ok(_) => match engine.reorder_channels(&server_id, &channels).await {
                    Ok(()) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::ChannelReorder {
                                server_id,
                                channels,
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        // ── Phase 4: Presence ──
        ClientMessage::SetPresence {
            status,
            custom_status,
            status_emoji,
        } => {
            engine
                .set_presence(
                    session_id,
                    &status,
                    custom_status.as_deref(),
                    status_emoji.as_deref(),
                )
                .await
        }
        ClientMessage::GetPresences { server_id } => {
            match engine.get_server_presences(&server_id).await {
                Ok(presences) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::PresenceList {
                            server_id,
                            presences,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        // ── Phase 4: Server Nicknames ──
        ClientMessage::SetServerNickname {
            server_id,
            nickname,
        } => {
            engine
                .set_server_nickname(session_id, &server_id, nickname.as_deref())
                .await
        }
        // ── Phase 4: Search ──
        ClientMessage::SearchMessages {
            server_id,
            query,
            channel,
            limit,
            offset,
        } => {
            let limit = limit.unwrap_or(25).min(50);
            let offset = offset.unwrap_or(0);
            match engine
                .search_messages(&server_id, &query, channel.as_deref(), limit, offset)
                .await
            {
                Ok((results, total_count)) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::SearchResults {
                            server_id,
                            query,
                            results,
                            total_count,
                            offset,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        // ── Phase 4: Notifications ──
        ClientMessage::UpdateNotificationSettings {
            server_id,
            channel_id,
            level,
            suppress_everyone,
            suppress_roles,
            muted,
            mute_until,
        } => {
            let params = crate::engine::chat_engine::UpdateNotificationSettingsParams {
                server_id: &server_id,
                channel_id: channel_id.as_deref(),
                level: &level,
                suppress_everyone: suppress_everyone.unwrap_or(false),
                suppress_roles: suppress_roles.unwrap_or(false),
                muted: muted.unwrap_or(false),
                mute_until: mute_until.as_deref(),
            };
            engine
                .update_notification_settings(session_id, &params)
                .await
        }
        ClientMessage::GetNotificationSettings { server_id } => {
            match engine
                .get_notification_settings(session_id, &server_id)
                .await
            {
                Ok(settings) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::NotificationSettings {
                            server_id,
                            settings,
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        // ── Phase 4: Profiles ──
        ClientMessage::GetUserProfile { user_id } => {
            match engine.get_user_profile(&user_id).await {
                Ok(profile) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::UserProfile { profile });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        // ── Phase 5: Pinning ──
        ClientMessage::PinMessage {
            server_id,
            channel,
            message_id,
        } => {
            engine
                .pin_message(session_id, &server_id, &channel, &message_id)
                .await
        }
        ClientMessage::UnpinMessage {
            server_id,
            channel,
            message_id,
        } => {
            engine
                .unpin_message(session_id, &server_id, &channel, &message_id)
                .await
        }
        ClientMessage::GetPinnedMessages { server_id, channel } => {
            engine
                .get_pinned_messages(session_id, &server_id, &channel)
                .await
        }
        // ── Phase 5: Threads ──
        ClientMessage::CreateThread {
            server_id,
            parent_channel,
            name,
            message_id,
            is_private,
        } => {
            engine
                .create_thread(
                    session_id,
                    &server_id,
                    &parent_channel,
                    &name,
                    &message_id,
                    is_private,
                )
                .await
        }
        ClientMessage::ArchiveThread {
            server_id,
            thread_id,
        } => {
            engine
                .archive_thread(session_id, &server_id, &thread_id)
                .await
        }
        ClientMessage::ListThreads { server_id, channel } => {
            engine.list_threads(session_id, &server_id, &channel).await
        }
        // ── Phase 5: Bookmarks ──
        ClientMessage::AddBookmark { message_id, note } => {
            engine
                .add_bookmark(session_id, &message_id, note.as_deref())
                .await
        }
        ClientMessage::RemoveBookmark { message_id } => {
            engine.remove_bookmark(session_id, &message_id).await
        }
        ClientMessage::ListBookmarks => engine.list_bookmarks(session_id).await,
        // ── Phase 6: Moderation ──
        ClientMessage::KickMember {
            server_id,
            user_id,
            reason,
        } => {
            engine
                .kick_member(session_id, &server_id, &user_id, reason.as_deref())
                .await
        }
        ClientMessage::BanMember {
            server_id,
            user_id,
            reason,
            delete_message_days,
        } => {
            engine
                .ban_member(
                    session_id,
                    &server_id,
                    &user_id,
                    reason.as_deref(),
                    delete_message_days,
                )
                .await
        }
        ClientMessage::UnbanMember { server_id, user_id } => {
            engine.unban_member(session_id, &server_id, &user_id).await
        }
        ClientMessage::ListBans { server_id } => engine.list_bans(session_id, &server_id).await,
        ClientMessage::TimeoutMember {
            server_id,
            user_id,
            timeout_until,
            reason,
        } => {
            engine
                .timeout_member(
                    session_id,
                    &server_id,
                    &user_id,
                    timeout_until.as_deref(),
                    reason.as_deref(),
                )
                .await
        }
        ClientMessage::SetSlowMode {
            server_id,
            channel,
            seconds,
        } => {
            engine
                .set_slowmode(session_id, &server_id, &channel, seconds)
                .await
        }
        ClientMessage::SetNsfw {
            server_id,
            channel,
            is_nsfw,
        } => {
            engine
                .set_nsfw(session_id, &server_id, &channel, is_nsfw)
                .await
        }
        ClientMessage::BulkDeleteMessages {
            server_id,
            channel,
            message_ids,
        } => {
            engine
                .bulk_delete_messages(session_id, &server_id, &channel, message_ids)
                .await
        }
        ClientMessage::GetAuditLog {
            server_id,
            action_type,
            limit,
            before,
        } => {
            let limit = limit.unwrap_or(50).clamp(1, 200);
            engine
                .get_audit_log(
                    session_id,
                    &server_id,
                    action_type.as_deref(),
                    limit,
                    before.as_deref(),
                )
                .await
        }
        // ── Phase 6: AutoMod ──
        ClientMessage::CreateAutomodRule {
            server_id,
            name,
            rule_type,
            config,
            action_type,
            timeout_duration_seconds,
        } => {
            let rule_id_placeholder = ""; // id generated inside engine
            engine
                .create_automod_rule(
                    session_id,
                    &crate::db::models::CreateAutomodRuleParams {
                        id: rule_id_placeholder,
                        server_id: &server_id,
                        name: &name,
                        rule_type: &rule_type,
                        config: &config,
                        action_type: &action_type,
                        timeout_duration_seconds,
                    },
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
            engine
                .update_automod_rule(
                    session_id,
                    &crate::db::models::UpdateAutomodRuleParams {
                        rule_id: &rule_id,
                        server_id: &server_id,
                        name: &name,
                        enabled,
                        config: &config,
                        action_type: &action_type,
                        timeout_duration_seconds,
                    },
                )
                .await
        }
        ClientMessage::DeleteAutomodRule { server_id, rule_id } => {
            engine
                .delete_automod_rule(session_id, &server_id, &rule_id)
                .await
        }
        ClientMessage::ListAutomodRules { server_id } => {
            engine.list_automod_rules(session_id, &server_id).await
        }
        // ── Phase 7: Community & Discovery ──
        ClientMessage::CreateInvite {
            server_id,
            max_uses,
            expires_at,
            channel_id,
        } => {
            engine
                .create_invite(
                    session_id,
                    &server_id,
                    max_uses,
                    expires_at.as_deref(),
                    channel_id.as_deref(),
                )
                .await
        }
        ClientMessage::ListInvites { server_id } => {
            engine.list_invites(session_id, &server_id).await
        }
        ClientMessage::DeleteInvite {
            server_id,
            invite_id,
        } => {
            engine
                .delete_invite(session_id, &server_id, &invite_id)
                .await
        }
        ClientMessage::UseInvite { code } => engine.use_invite(session_id, &code).await,
        ClientMessage::CreateEvent {
            server_id,
            name,
            description,
            channel_id,
            start_time,
            end_time,
            image_url,
        } => {
            let user_id = engine.get_session_user_id(session_id).unwrap_or_default();
            let event_id = uuid::Uuid::new_v4().to_string();
            engine
                .create_event(
                    session_id,
                    &crate::db::models::CreateServerEventParams {
                        id: &event_id,
                        server_id: &server_id,
                        name: &name,
                        description: description.as_deref(),
                        channel_id: channel_id.as_deref(),
                        start_time: &start_time,
                        end_time: end_time.as_deref(),
                        image_url: image_url.as_deref(),
                        created_by: &user_id,
                    },
                )
                .await
        }
        ClientMessage::ListEvents { server_id } => engine.list_events(session_id, &server_id).await,
        ClientMessage::UpdateEventStatus {
            server_id,
            event_id,
            status,
        } => {
            engine
                .update_event_status(session_id, &server_id, &event_id, &status)
                .await
        }
        ClientMessage::DeleteEvent {
            server_id,
            event_id,
        } => engine.delete_event(session_id, &server_id, &event_id).await,
        ClientMessage::SetRsvp {
            server_id,
            event_id,
            status,
        } => {
            engine
                .set_rsvp(session_id, &server_id, &event_id, &status)
                .await
        }
        ClientMessage::RemoveRsvp {
            server_id,
            event_id,
        } => engine.remove_rsvp(session_id, &server_id, &event_id).await,
        ClientMessage::ListRsvps { event_id } => engine.list_rsvps(session_id, &event_id).await,
        ClientMessage::UpdateCommunitySettings {
            server_id,
            description,
            is_discoverable,
            welcome_message,
            rules_text,
            category,
        } => {
            engine
                .update_community_settings(
                    session_id,
                    &server_id,
                    description.as_deref(),
                    is_discoverable,
                    welcome_message.as_deref(),
                    rules_text.as_deref(),
                    category.as_deref(),
                )
                .await
        }
        ClientMessage::GetCommunitySettings { server_id } => {
            engine.get_community_settings(session_id, &server_id).await
        }
        ClientMessage::DiscoverServers { category } => {
            engine
                .discover_servers(session_id, category.as_deref())
                .await
        }
        ClientMessage::AcceptRules { server_id } => {
            engine.accept_rules(session_id, &server_id).await
        }
        ClientMessage::SetAnnouncementChannel {
            server_id,
            channel,
            is_announcement,
        } => {
            engine
                .set_announcement_channel(session_id, &server_id, &channel, is_announcement)
                .await
        }
        ClientMessage::FollowChannel {
            source_channel_id,
            target_channel_id,
        } => {
            engine
                .follow_channel(session_id, &source_channel_id, &target_channel_id)
                .await
        }
        ClientMessage::UnfollowChannel { follow_id } => {
            engine.unfollow_channel(session_id, &follow_id).await
        }
        ClientMessage::ListChannelFollows { channel_id } => {
            engine.list_channel_follows(session_id, &channel_id).await
        }
        ClientMessage::CreateTemplate {
            server_id,
            name,
            description,
        } => {
            engine
                .create_template(session_id, &server_id, &name, description.as_deref())
                .await
        }
        ClientMessage::ListTemplates { server_id } => {
            engine.list_templates(session_id, &server_id).await
        }
        ClientMessage::DeleteTemplate {
            server_id,
            template_id,
        } => {
            engine
                .delete_template(session_id, &server_id, &template_id)
                .await
        }
        // ── Phase 8: Integrations & Bots ──
        ClientMessage::CreateWebhook {
            server_id,
            channel_id,
            name,
            webhook_type,
            url,
        } => {
            engine
                .create_webhook(
                    session_id,
                    &server_id,
                    &channel_id,
                    &name,
                    &webhook_type,
                    url.as_deref(),
                )
                .await
        }
        ClientMessage::ListWebhooks { server_id } => {
            engine.list_webhooks(session_id, &server_id).await
        }
        ClientMessage::UpdateWebhook {
            webhook_id,
            name,
            avatar_url,
            channel_id,
        } => {
            engine
                .update_webhook(
                    session_id,
                    &webhook_id,
                    &name,
                    avatar_url.as_deref(),
                    &channel_id,
                )
                .await
        }
        ClientMessage::DeleteWebhook { webhook_id } => {
            engine.delete_webhook(session_id, &webhook_id).await
        }
        ClientMessage::CreateBot {
            username,
            avatar_url,
        } => {
            engine
                .create_bot(session_id, &username, avatar_url.as_deref())
                .await
        }
        ClientMessage::CreateBotToken {
            bot_user_id,
            name,
            scopes,
        } => {
            engine
                .create_bot_token(session_id, &bot_user_id, &name, scopes.as_deref())
                .await
        }
        ClientMessage::ListBotTokens { bot_user_id } => {
            engine.list_bot_tokens(session_id, &bot_user_id).await
        }
        ClientMessage::DeleteBotToken { token_id } => {
            engine.delete_bot_token(session_id, &token_id).await
        }
        ClientMessage::AddBotToServer {
            server_id,
            bot_user_id,
        } => {
            engine
                .add_bot_to_server(session_id, &server_id, &bot_user_id)
                .await
        }
        ClientMessage::RemoveBotFromServer {
            server_id,
            bot_user_id,
        } => {
            engine
                .remove_bot_from_server(session_id, &server_id, &bot_user_id)
                .await
        }
        ClientMessage::RegisterSlashCommand {
            server_id,
            name,
            description,
            options_json,
        } => {
            engine
                .register_slash_command(
                    session_id,
                    &server_id,
                    &name,
                    &description,
                    options_json.as_deref(),
                )
                .await
        }
        ClientMessage::ListSlashCommands { server_id } => {
            engine.list_slash_commands(session_id, &server_id).await
        }
        ClientMessage::DeleteSlashCommand { command_id } => {
            engine.delete_slash_command(session_id, &command_id).await
        }
        ClientMessage::InvokeSlashCommand {
            server_id,
            channel,
            command_name,
            args_json,
        } => {
            engine
                .invoke_slash_command(
                    session_id,
                    &server_id,
                    &channel,
                    &command_name,
                    args_json.as_deref(),
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
            engine
                .respond_to_interaction(
                    session_id,
                    &interaction_id,
                    content.as_deref(),
                    embeds_json.as_deref(),
                    components_json.as_deref(),
                    ephemeral.unwrap_or(false),
                )
                .await
        }
        ClientMessage::CreateOAuth2App {
            name,
            description,
            redirect_uris,
        } => {
            engine
                .create_oauth2_app(session_id, &name, description.as_deref(), &redirect_uris)
                .await
        }
        ClientMessage::ListOAuth2Apps => engine.list_oauth2_apps(session_id).await,
        ClientMessage::DeleteOAuth2App { app_id } => {
            engine.delete_oauth2_app(session_id, &app_id).await
        }

        // ── Premium-for-Free features ──
        ClientMessage::SetServerAvatar {
            server_id,
            avatar_url,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_SERVER,
                )
                .await
            {
                Err(e) => Err(e),
                Ok(_) => {
                    let user_id = engine
                        .get_session_user_id(session_id)
                        .ok_or_else(|| "Not authenticated".to_string());
                    match user_id {
                        Err(e) => Err(e),
                        Ok(user_id) => {
                            let pool = engine.get_db().ok_or("No database".to_string());
                            match pool {
                                Err(e) => Err(e),
                                Ok(pool) => {
                                    match crate::db::queries::servers::set_server_avatar(
                                        &pool,
                                        &server_id,
                                        &user_id,
                                        avatar_url.as_deref(),
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            let event = ChatEvent::ServerAvatarUpdate {
                                                server_id: server_id.clone(),
                                                user_id,
                                                avatar_url,
                                            };
                                            engine.broadcast_to_server(&server_id, &event);
                                            Ok(())
                                        }
                                        Err(e) => Err(format!("Failed to set server avatar: {e}")),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        ClientMessage::SetVanityCode {
            server_id,
            vanity_code,
        } => {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_SERVER,
                )
                .await
            {
                Err(e) => Err(e),
                Ok(_) => {
                    let valid = if let Some(ref code) = vanity_code {
                        crate::engine::validation::validate_vanity_code(code)
                    } else {
                        Ok(())
                    };
                    match valid {
                        Err(e) => Err(e),
                        Ok(()) => match engine.get_db() {
                            None => Err("No database".to_string()),
                            Some(pool) => {
                                match crate::db::queries::servers::set_vanity_code(
                                    &pool,
                                    &server_id,
                                    vanity_code.as_deref(),
                                )
                                .await
                                {
                                    Ok(()) => Ok(()),
                                    Err(e) => Err(format!("Failed to set vanity code: {e}")),
                                }
                            }
                        },
                    }
                }
            }
        }

        ClientMessage::GetServerLimits => {
            if let Some(session) = engine.get_session(session_id) {
                let _ = session.send(ChatEvent::ServerLimits {
                    max_message_length: engine.max_message_length(),
                    max_file_size_mb: engine.max_file_size_mb(),
                });
            }
            Ok(())
        }
    };

    if let Err(e) = result {
        send_error(engine, session_id, "COMMAND_FAILED", &e);
    }
}

fn send_error(
    engine: &ChatEngine,
    session_id: crate::engine::events::SessionId,
    code: &str,
    message: &str,
) {
    if let Some(session) = engine.get_session(session_id) {
        let _ = session.send(ChatEvent::Error {
            code: code.into(),
            message: message.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to deserialize a JSON string into a ClientMessage.
    fn parse_msg(json: &str) -> Result<ClientMessage, serde_json::Error> {
        serde_json::from_str(json)
    }

    // ── Core messaging ──

    #[test]
    fn test_send_message_basic() {
        let msg: ClientMessage = parse_msg(
            r##"{"type": "send_message", "channel": "#general", "content": "Hello world"}"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SendMessage {
                server_id,
                channel,
                content,
                reply_to,
                attachment_ids,
                nonce,
            } => {
                assert_eq!(server_id, DEFAULT_SERVER_ID);
                assert_eq!(channel, "#general");
                assert_eq!(content, "Hello world");
                assert!(reply_to.is_none());
                assert!(attachment_ids.is_none());
                assert!(nonce.is_none());
            }
            _ => panic!("Expected SendMessage"),
        }
    }

    #[test]
    fn test_send_message_with_reply_and_attachments() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "send_message",
            "server_id": "srv-1",
            "channel": "#dev",
            "content": "See attached",
            "reply_to": "msg-123",
            "attachment_ids": ["att-1", "att-2"]
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SendMessage {
                server_id,
                reply_to,
                attachment_ids,
                ..
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(reply_to, Some("msg-123".into()));
                assert_eq!(attachment_ids, Some(vec!["att-1".into(), "att-2".into()]));
            }
            _ => panic!("Expected SendMessage"),
        }
    }

    #[test]
    fn test_join_channel() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "join_channel",
            "server_id": "srv-1",
            "channel": "#random"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::JoinChannel { server_id, channel } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, "#random");
            }
            _ => panic!("Expected JoinChannel"),
        }
    }

    #[test]
    fn test_join_channel_default_server() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "join_channel",
            "channel": "#random"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::JoinChannel { server_id, .. } => {
                assert_eq!(server_id, DEFAULT_SERVER_ID);
            }
            _ => panic!("Expected JoinChannel"),
        }
    }

    #[test]
    fn test_part_channel() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "part_channel",
            "server_id": "srv-1",
            "channel": "#random",
            "reason": "Going offline"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::PartChannel {
                server_id,
                channel,
                reason,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, "#random");
                assert_eq!(reason, Some("Going offline".into()));
            }
            _ => panic!("Expected PartChannel"),
        }
    }

    #[test]
    fn test_set_topic() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "set_topic",
            "server_id": "srv-1",
            "channel": "#general",
            "topic": "New topic"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SetTopic { topic, .. } => {
                assert_eq!(topic, "New topic");
            }
            _ => panic!("Expected SetTopic"),
        }
    }

    #[test]
    fn test_fetch_history() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "fetch_history",
            "server_id": "srv-1",
            "channel": "#general",
            "before": "2025-01-01T00:00:00Z",
            "limit": 25
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::FetchHistory { before, limit, .. } => {
                assert_eq!(before, Some("2025-01-01T00:00:00Z".into()));
                assert_eq!(limit, Some(25));
            }
            _ => panic!("Expected FetchHistory"),
        }
    }

    #[test]
    fn test_fetch_history_defaults() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "fetch_history",
            "channel": "#general"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::FetchHistory {
                server_id,
                before,
                limit,
                ..
            } => {
                assert_eq!(server_id, DEFAULT_SERVER_ID);
                assert!(before.is_none());
                assert!(limit.is_none());
            }
            _ => panic!("Expected FetchHistory"),
        }
    }

    #[test]
    fn test_list_channels() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "list_channels",
            "server_id": "srv-1"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::ListChannels { server_id } => {
                assert_eq!(server_id, "srv-1");
            }
            _ => panic!("Expected ListChannels"),
        }
    }

    #[test]
    fn test_list_servers() {
        let msg: ClientMessage = parse_msg(r##"{"type": "list_servers"}"##).unwrap();
        assert!(matches!(msg, ClientMessage::ListServers));
    }

    // ── Server management ──

    #[test]
    fn test_create_server() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_server",
            "name": "My Server",
            "icon_url": "https://example.com/icon.png"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateServer { name, icon_url } => {
                assert_eq!(name, "My Server");
                assert_eq!(icon_url, Some("https://example.com/icon.png".into()));
            }
            _ => panic!("Expected CreateServer"),
        }
    }

    #[test]
    fn test_create_server_no_icon() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_server",
            "name": "My Server"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateServer { name, icon_url } => {
                assert_eq!(name, "My Server");
                assert!(icon_url.is_none());
            }
            _ => panic!("Expected CreateServer"),
        }
    }

    #[test]
    fn test_join_server() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "join_server",
            "server_id": "srv-1"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::JoinServer { server_id } if server_id == "srv-1"));
    }

    #[test]
    fn test_leave_server() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "leave_server",
            "server_id": "srv-1"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::LeaveServer { server_id } if server_id == "srv-1"));
    }

    #[test]
    fn test_delete_server() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "delete_server",
            "server_id": "srv-1"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::DeleteServer { server_id } if server_id == "srv-1"));
    }

    #[test]
    fn test_create_channel() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_channel",
            "server_id": "srv-1",
            "name": "new-channel"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateChannel {
                server_id,
                name,
                category_id,
                is_private,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(name, "new-channel");
                assert!(category_id.is_none());
                assert!(is_private.is_none());
            }
            _ => panic!("Expected CreateChannel"),
        }
    }

    #[test]
    fn test_delete_channel() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "delete_channel",
            "server_id": "srv-1",
            "channel": "#old"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::DeleteChannel { server_id, channel } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, "#old");
            }
            _ => panic!("Expected DeleteChannel"),
        }
    }

    // ── Message actions ──

    #[test]
    fn test_edit_message() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "edit_message",
            "message_id": "msg-1",
            "content": "edited content"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::EditMessage {
                message_id,
                content,
            } => {
                assert_eq!(message_id, "msg-1");
                assert_eq!(content, "edited content");
            }
            _ => panic!("Expected EditMessage"),
        }
    }

    #[test]
    fn test_delete_message() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "delete_message",
            "message_id": "msg-1"
        }"##,
        )
        .unwrap();
        assert!(
            matches!(msg, ClientMessage::DeleteMessage { message_id } if message_id == "msg-1")
        );
    }

    #[test]
    fn test_add_reaction() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "add_reaction",
            "message_id": "msg-1",
            "emoji": "\ud83d\udc4d"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::AddReaction { message_id, emoji } => {
                assert_eq!(message_id, "msg-1");
                assert_eq!(emoji, "\u{1f44d}");
            }
            _ => panic!("Expected AddReaction"),
        }
    }

    #[test]
    fn test_remove_reaction() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "remove_reaction",
            "message_id": "msg-1",
            "emoji": "\ud83d\udc4d"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::RemoveReaction { .. }));
    }

    #[test]
    fn test_typing() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "typing",
            "channel": "#general"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::Typing { server_id, channel } => {
                assert_eq!(server_id, DEFAULT_SERVER_ID);
                assert_eq!(channel, "#general");
            }
            _ => panic!("Expected Typing"),
        }
    }

    #[test]
    fn test_mark_read() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "mark_read",
            "server_id": "srv-1",
            "channel": "#general",
            "message_id": "msg-42"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::MarkRead {
                server_id,
                channel,
                message_id,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, "#general");
                assert_eq!(message_id, "msg-42");
            }
            _ => panic!("Expected MarkRead"),
        }
    }

    // ── Roles ──

    #[test]
    fn test_list_roles() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "list_roles",
            "server_id": "srv-1"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::ListRoles { server_id } if server_id == "srv-1"));
    }

    #[test]
    fn test_create_role() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_role",
            "server_id": "srv-1",
            "name": "Moderator",
            "color": "#ff0000",
            "permissions": 42
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateRole {
                server_id,
                name,
                color,
                permissions,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(name, "Moderator");
                assert_eq!(color, Some("#ff0000".into()));
                assert_eq!(permissions, Some(42));
            }
            _ => panic!("Expected CreateRole"),
        }
    }

    #[test]
    fn test_create_role_defaults() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_role",
            "server_id": "srv-1",
            "name": "Basic"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateRole {
                color, permissions, ..
            } => {
                assert!(color.is_none());
                assert!(permissions.is_none());
            }
            _ => panic!("Expected CreateRole"),
        }
    }

    // ── Categories ──

    #[test]
    fn test_create_category() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_category",
            "server_id": "srv-1",
            "name": "Text Channels"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateCategory { server_id, name } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(name, "Text Channels");
            }
            _ => panic!("Expected CreateCategory"),
        }
    }

    // ── Presence ──

    #[test]
    fn test_set_presence() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "set_presence",
            "status": "dnd",
            "custom_status": "In a meeting",
            "status_emoji": "\ud83d\udcbc"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SetPresence {
                status,
                custom_status,
                status_emoji,
            } => {
                assert_eq!(status, "dnd");
                assert_eq!(custom_status, Some("In a meeting".into()));
                assert_eq!(status_emoji, Some("\u{1f4bc}".into()));
            }
            _ => panic!("Expected SetPresence"),
        }
    }

    // ── Phase 5: Threads ──

    #[test]
    fn test_create_thread() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_thread",
            "server_id": "srv-1",
            "parent_channel": "#general",
            "name": "Discussion",
            "message_id": "msg-1",
            "is_private": true
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateThread {
                server_id,
                parent_channel,
                name,
                message_id,
                is_private,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(parent_channel, "#general");
                assert_eq!(name, "Discussion");
                assert_eq!(message_id, "msg-1");
                assert!(is_private);
            }
            _ => panic!("Expected CreateThread"),
        }
    }

    #[test]
    fn test_create_thread_defaults() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_thread",
            "server_id": "srv-1",
            "parent_channel": "#general",
            "name": "Public Thread",
            "message_id": "msg-2"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateThread { is_private, .. } => {
                assert!(!is_private);
            }
            _ => panic!("Expected CreateThread"),
        }
    }

    // ── Phase 5: Bookmarks ──

    #[test]
    fn test_add_bookmark() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "add_bookmark",
            "message_id": "msg-1",
            "note": "Important info"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::AddBookmark { message_id, note } => {
                assert_eq!(message_id, "msg-1");
                assert_eq!(note, Some("Important info".into()));
            }
            _ => panic!("Expected AddBookmark"),
        }
    }

    #[test]
    fn test_list_bookmarks() {
        let msg: ClientMessage = parse_msg(r##"{"type": "list_bookmarks"}"##).unwrap();
        assert!(matches!(msg, ClientMessage::ListBookmarks));
    }

    // ── Phase 6: Moderation ──

    #[test]
    fn test_kick_member() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "kick_member",
            "server_id": "srv-1",
            "user_id": "user-1",
            "reason": "Spamming"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::KickMember {
                server_id,
                user_id,
                reason,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(user_id, "user-1");
                assert_eq!(reason, Some("Spamming".into()));
            }
            _ => panic!("Expected KickMember"),
        }
    }

    #[test]
    fn test_ban_member() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "ban_member",
            "server_id": "srv-1",
            "user_id": "user-1",
            "reason": "Harassment",
            "delete_message_days": 7
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::BanMember {
                server_id,
                user_id,
                reason,
                delete_message_days,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(user_id, "user-1");
                assert_eq!(reason, Some("Harassment".into()));
                assert_eq!(delete_message_days, 7);
            }
            _ => panic!("Expected BanMember"),
        }
    }

    #[test]
    fn test_ban_member_defaults() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "ban_member",
            "server_id": "srv-1",
            "user_id": "user-1"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::BanMember {
                delete_message_days,
                reason,
                ..
            } => {
                assert_eq!(delete_message_days, 0);
                assert!(reason.is_none());
            }
            _ => panic!("Expected BanMember"),
        }
    }

    #[test]
    fn test_set_slow_mode() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "set_slow_mode",
            "server_id": "srv-1",
            "channel": "#general",
            "seconds": 10
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SetSlowMode { seconds, .. } => {
                assert_eq!(seconds, 10);
            }
            _ => panic!("Expected SetSlowMode"),
        }
    }

    #[test]
    fn test_bulk_delete() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "bulk_delete_messages",
            "server_id": "srv-1",
            "channel": "#general",
            "message_ids": ["m1", "m2", "m3"]
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::BulkDeleteMessages { message_ids, .. } => {
                assert_eq!(message_ids.len(), 3);
            }
            _ => panic!("Expected BulkDeleteMessages"),
        }
    }

    // ── Phase 7: Community ──

    #[test]
    fn test_create_invite() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_invite",
            "server_id": "srv-1",
            "max_uses": 10,
            "expires_at": "2026-12-31T23:59:59Z"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateInvite {
                server_id,
                max_uses,
                expires_at,
                channel_id,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(max_uses, Some(10));
                assert_eq!(expires_at, Some("2026-12-31T23:59:59Z".into()));
                assert!(channel_id.is_none());
            }
            _ => panic!("Expected CreateInvite"),
        }
    }

    #[test]
    fn test_use_invite() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "use_invite",
            "code": "abc123"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::UseInvite { code } if code == "abc123"));
    }

    #[test]
    fn test_create_event() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_event",
            "server_id": "srv-1",
            "name": "Game Night",
            "description": "Playing board games",
            "start_time": "2026-03-01T19:00:00Z"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateEvent {
                name,
                description,
                start_time,
                end_time,
                ..
            } => {
                assert_eq!(name, "Game Night");
                assert_eq!(description, Some("Playing board games".into()));
                assert_eq!(start_time, "2026-03-01T19:00:00Z");
                assert!(end_time.is_none());
            }
            _ => panic!("Expected CreateEvent"),
        }
    }

    #[test]
    fn test_discover_servers() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "discover_servers",
            "category": "gaming"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::DiscoverServers { category } => {
                assert_eq!(category, Some("gaming".into()));
            }
            _ => panic!("Expected DiscoverServers"),
        }
    }

    // ── Phase 8: Integrations & Bots ──

    #[test]
    fn test_create_webhook() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_webhook",
            "server_id": "srv-1",
            "channel_id": "ch-1",
            "name": "GitHub Notifications",
            "webhook_type": "incoming",
            "url": "https://example.com/hook"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateWebhook {
                server_id,
                channel_id,
                name,
                webhook_type,
                url,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel_id, "ch-1");
                assert_eq!(name, "GitHub Notifications");
                assert_eq!(webhook_type, "incoming");
                assert_eq!(url, Some("https://example.com/hook".into()));
            }
            _ => panic!("Expected CreateWebhook"),
        }
    }

    #[test]
    fn test_create_bot() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_bot",
            "username": "mybot",
            "avatar_url": "https://example.com/bot.png"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateBot {
                username,
                avatar_url,
            } => {
                assert_eq!(username, "mybot");
                assert_eq!(avatar_url, Some("https://example.com/bot.png".into()));
            }
            _ => panic!("Expected CreateBot"),
        }
    }

    #[test]
    fn test_create_bot_token() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_bot_token",
            "bot_user_id": "bot-1",
            "name": "production",
            "scopes": "read,write"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateBotToken {
                bot_user_id,
                name,
                scopes,
            } => {
                assert_eq!(bot_user_id, "bot-1");
                assert_eq!(name, "production");
                assert_eq!(scopes, Some("read,write".into()));
            }
            _ => panic!("Expected CreateBotToken"),
        }
    }

    #[test]
    fn test_register_slash_command() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "register_slash_command",
            "server_id": "srv-1",
            "name": "ping",
            "description": "Check if bot is alive",
            "options_json": "[{\"name\":\"target\",\"type\":\"string\"}]"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::RegisterSlashCommand {
                server_id,
                name,
                description,
                options_json,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(name, "ping");
                assert_eq!(description, "Check if bot is alive");
                assert!(options_json.is_some());
            }
            _ => panic!("Expected RegisterSlashCommand"),
        }
    }

    #[test]
    fn test_invoke_slash_command() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "invoke_slash_command",
            "server_id": "srv-1",
            "channel": "#general",
            "command_name": "ping"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::InvokeSlashCommand {
                command_name,
                args_json,
                ..
            } => {
                assert_eq!(command_name, "ping");
                assert!(args_json.is_none());
            }
            _ => panic!("Expected InvokeSlashCommand"),
        }
    }

    #[test]
    fn test_respond_to_interaction() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "respond_to_interaction",
            "interaction_id": "int-1",
            "content": "Pong!",
            "ephemeral": true
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::RespondToInteraction {
                interaction_id,
                content,
                ephemeral,
                ..
            } => {
                assert_eq!(interaction_id, "int-1");
                assert_eq!(content, Some("Pong!".into()));
                assert_eq!(ephemeral, Some(true));
            }
            _ => panic!("Expected RespondToInteraction"),
        }
    }

    #[test]
    fn test_create_oauth2_app() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_o_auth2_app",
            "name": "My App",
            "description": "A cool app",
            "redirect_uris": ["https://example.com/callback"]
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateOAuth2App {
                name,
                description,
                redirect_uris,
            } => {
                assert_eq!(name, "My App");
                assert_eq!(description, Some("A cool app".into()));
                assert_eq!(redirect_uris, vec!["https://example.com/callback"]);
            }
            _ => panic!("Expected CreateOAuth2App"),
        }
    }

    #[test]
    fn test_list_oauth2_apps() {
        let msg: ClientMessage = parse_msg(r##"{"type": "list_o_auth2_apps"}"##).unwrap();
        assert!(matches!(msg, ClientMessage::ListOAuth2Apps));
    }

    // ── Malformed JSON handling ──

    #[test]
    fn test_malformed_json_completely_invalid() {
        assert!(parse_msg("not json at all").is_err());
    }

    #[test]
    fn test_malformed_json_missing_type() {
        assert!(parse_msg(r##"{"channel": "#general"}"##).is_err());
    }

    #[test]
    fn test_malformed_json_unknown_type() {
        assert!(parse_msg(r##"{"type": "unknown_command"}"##).is_err());
    }

    #[test]
    fn test_malformed_json_missing_required_field() {
        // SendMessage requires channel and content
        assert!(parse_msg(r##"{"type": "send_message"}"##).is_err());
    }

    #[test]
    fn test_malformed_json_wrong_field_type() {
        // limit should be a number, not a string
        assert!(
            parse_msg(
                r##"{
            "type": "fetch_history",
            "channel": "#general",
            "limit": "not a number"
        }"##
            )
            .is_err()
        );
    }

    #[test]
    fn test_extra_fields_ignored() {
        // Extra fields should be silently ignored by serde
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "list_servers",
            "unknown_field": "should be ignored",
            "another_extra": 42
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::ListServers));
    }

    #[test]
    fn test_empty_json_object() {
        assert!(parse_msg("{}").is_err());
    }

    #[test]
    fn test_null_type() {
        assert!(parse_msg(r##"{"type": null}"##).is_err());
    }

    // ── Default server_id function ──

    #[test]
    fn test_default_server_id() {
        assert_eq!(default_server_id(), DEFAULT_SERVER_ID);
    }

    // ── Additional moderation commands ──

    #[test]
    fn test_set_nsfw() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "set_nsfw",
            "server_id": "srv-1",
            "channel": "#mature",
            "is_nsfw": true
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SetNsfw { is_nsfw, .. } => {
                assert!(is_nsfw);
            }
            _ => panic!("Expected SetNsfw"),
        }
    }

    #[test]
    fn test_get_audit_log() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "get_audit_log",
            "server_id": "srv-1",
            "action_type": "ban",
            "limit": 25
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::GetAuditLog {
                server_id,
                action_type,
                limit,
                before,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(action_type, Some("ban".into()));
                assert_eq!(limit, Some(25));
                assert!(before.is_none());
            }
            _ => panic!("Expected GetAuditLog"),
        }
    }

    #[test]
    fn test_create_automod_rule() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_automod_rule",
            "server_id": "srv-1",
            "name": "No Spam",
            "rule_type": "keyword",
            "config": "{\"keywords\":[\"spam\"]}",
            "action_type": "delete"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateAutomodRule {
                name,
                rule_type,
                action_type,
                timeout_duration_seconds,
                ..
            } => {
                assert_eq!(name, "No Spam");
                assert_eq!(rule_type, "keyword");
                assert_eq!(action_type, "delete");
                assert!(timeout_duration_seconds.is_none());
            }
            _ => panic!("Expected CreateAutomodRule"),
        }
    }

    // ── Community features ──

    #[test]
    fn test_update_community_settings() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "update_community_settings",
            "server_id": "srv-1",
            "description": "A cool server",
            "is_discoverable": true,
            "welcome_message": "Welcome!",
            "rules_text": "Be nice",
            "category": "gaming"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::UpdateCommunitySettings {
                is_discoverable,
                category,
                ..
            } => {
                assert!(is_discoverable);
                assert_eq!(category, Some("gaming".into()));
            }
            _ => panic!("Expected UpdateCommunitySettings"),
        }
    }

    #[test]
    fn test_follow_channel() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "follow_channel",
            "source_channel_id": "ch-1",
            "target_channel_id": "ch-2"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::FollowChannel {
                source_channel_id,
                target_channel_id,
            } => {
                assert_eq!(source_channel_id, "ch-1");
                assert_eq!(target_channel_id, "ch-2");
            }
            _ => panic!("Expected FollowChannel"),
        }
    }

    #[test]
    fn test_create_template() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "create_template",
            "server_id": "srv-1",
            "name": "Gaming Server",
            "description": "A template for gaming servers"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::CreateTemplate {
                name, description, ..
            } => {
                assert_eq!(name, "Gaming Server");
                assert_eq!(description, Some("A template for gaming servers".into()));
            }
            _ => panic!("Expected CreateTemplate"),
        }
    }

    // ── Pin/Unpin ──

    #[test]
    fn test_pin_message() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "pin_message",
            "server_id": "srv-1",
            "channel": "#general",
            "message_id": "msg-1"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::PinMessage {
                server_id,
                channel,
                message_id,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, "#general");
                assert_eq!(message_id, "msg-1");
            }
            _ => panic!("Expected PinMessage"),
        }
    }

    #[test]
    fn test_unpin_message() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "unpin_message",
            "server_id": "srv-1",
            "channel": "#general",
            "message_id": "msg-1"
        }"##,
        )
        .unwrap();
        assert!(matches!(msg, ClientMessage::UnpinMessage { .. }));
    }

    // ── Search ──

    #[test]
    fn test_search_messages() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "search_messages",
            "server_id": "srv-1",
            "query": "hello world",
            "channel": "#general",
            "limit": 10,
            "offset": 5
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::SearchMessages {
                query,
                channel,
                limit,
                offset,
                ..
            } => {
                assert_eq!(query, "hello world");
                assert_eq!(channel, Some("#general".into()));
                assert_eq!(limit, Some(10));
                assert_eq!(offset, Some(5));
            }
            _ => panic!("Expected SearchMessages"),
        }
    }

    // ── Notifications ──

    #[test]
    fn test_update_notification_settings() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "update_notification_settings",
            "server_id": "srv-1",
            "level": "mentions_only",
            "suppress_everyone": true,
            "muted": false
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::UpdateNotificationSettings {
                level,
                suppress_everyone,
                muted,
                ..
            } => {
                assert_eq!(level, "mentions_only");
                assert_eq!(suppress_everyone, Some(true));
                assert_eq!(muted, Some(false));
            }
            _ => panic!("Expected UpdateNotificationSettings"),
        }
    }
}
