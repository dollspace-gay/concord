use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Extension, State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::auth::authority::{Actor, AuthService};
use crate::db::queries::users;
use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::ChatEvent;
use crate::engine::permissions::Permissions;
use crate::engine::user_session::Protocol;

use super::app_state::AppState;

fn default_oauth_client_type() -> String {
    "confidential".to_owned()
}

/// Client-to-server WebSocket message types.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientMessage {
    /// Explicit correlation envelope for non-durable lifecycle commands.
    LifecycleCommand {
        request_id: String,
        command: Box<ClientMessage>,
    },
    Sync {
        request_id: String,
        protocol_version: u32,
        subscriptions: Vec<String>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    SendMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        content: String,
        #[serde(default)]
        content_format: crate::engine::messaging::ContentFormat,
        reply_to: Option<String>,
        attachment_ids: Option<Vec<String>>,
        #[serde(default)]
        mentions: Vec<crate::engine::messaging::MessageMention>,
        nonce: Option<String>,
    },
    SendDirectMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        recipient: String,
        content: String,
        #[serde(default)]
        content_format: crate::engine::messaging::ContentFormat,
        reply_to: Option<String>,
        attachment_ids: Option<Vec<String>>,
        nonce: Option<String>,
    },
    ListDirectConversations,
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
        channel_type: Option<String>,
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
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
        content: String,
        #[serde(default)]
        content_format: crate::engine::messaging::ContentFormat,
        #[serde(default)]
        mentions: Vec<crate::engine::messaging::MessageMention>,
    },
    DeleteMessage {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
    },
    AddReaction {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
        emoji: String,
    },
    RemoveReaction {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        message_id: String,
        emoji: String,
    },
    Typing {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
    },
    MarkRead {
        operation_generation: String,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        #[serde(default)]
        conversation_id: Option<String>,
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
    ListChannelPermissionOverrides {
        server_id: String,
        channel_id: String,
    },
    SetChannelPermissionOverride {
        server_id: String,
        channel_id: String,
        target_type: String,
        target_id: String,
        allow_bits: i64,
        deny_bits: i64,
    },
    DeleteChannelPermissionOverride {
        server_id: String,
        channel_id: String,
        target_type: String,
        target_id: String,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        server_id: String,
        query: String,
        channel: Option<String>,
        limit: Option<i64>,
        offset: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<String>,
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
    UnarchiveThread {
        server_id: String,
        thread_id: String,
    },
    ListThreads {
        server_id: String,
        channel: String,
    },
    CreateForumTag {
        server_id: String,
        channel: String,
        name: String,
        emoji: Option<String>,
        #[serde(default)]
        moderated: bool,
    },
    UpdateForumTag {
        server_id: String,
        channel: String,
        tag_id: String,
        name: String,
        emoji: Option<String>,
        moderated: bool,
        position: i32,
    },
    DeleteForumTag {
        server_id: String,
        channel: String,
        tag_id: String,
    },
    ListForumTags {
        server_id: String,
        channel: String,
    },
    SetThreadTags {
        server_id: String,
        thread_id: String,
        tag_ids: Vec<String>,
    },
    GetThreadTags {
        server_id: String,
        thread_id: String,
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
    PublishAnnouncement {
        message_id: String,
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
    InstantiateTemplate {
        template_id: String,
        server_name: String,
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
    ListOwnedBots,
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
        request_id: String,
        server_id: String,
        channel: String,
        command_name: String,
        args_json: Option<String>,
    },
    InvokeMessageComponent {
        request_id: String,
        message_id: String,
        custom_id: String,
        #[serde(default)]
        values: Vec<String>,
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
        #[serde(default = "default_oauth_client_type")]
        client_type: String,
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
    Extension(rate_limiters): Extension<Arc<super::rate_limit::ApiRateLimiters>>,
    jar: CookieJar,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let expected_origin = state.auth_config.public_url.trim_end_matches('/');
    if headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        != Some(expected_origin)
    {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Invalid WebSocket origin",
        )
            .into_response();
    }
    // Browser sessions use the same-site cookie. Bot clients use the canonical
    // bearer credential rather than trying to impersonate a browser session.
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if jar.get("concord_session").is_some() && bearer.is_some() {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Provide exactly one WebSocket credential",
        )
            .into_response();
    }
    let (nickname, actor, avatar_url) = if let Some(cookie) = jar.get("concord_session") {
        let actor = match state.auth.authenticate_web_session(cookie.value()).await {
            Ok(actor) => actor,
            Err(error) => {
                return super::auth_middleware::auth_error_response(error, "Invalid session token");
            }
        };
        match users::get_user(&state.db, actor.user_id().as_str()).await {
            Ok(Some((_id, username, _email, avatar))) => (username, actor, avatar),
            Ok(None) => {
                return (axum::http::StatusCode::UNAUTHORIZED, "User not found").into_response();
            }
            Err(_) => {
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Authentication service unavailable",
                )
                    .into_response();
            }
        }
    } else if let Some(token) = bearer {
        let actor = match state.auth.authenticate_bot(token).await {
            Ok(actor) => actor,
            Err(error) => {
                return super::auth_middleware::auth_error_response(error, "Invalid bot token");
            }
        };
        match users::get_user(&state.db, actor.user_id().as_str()).await {
            Ok(Some((_id, username, _email, avatar))) => (username, actor, avatar),
            Ok(None) => {
                return (axum::http::StatusCode::UNAUTHORIZED, "Bot user not found")
                    .into_response();
            }
            Err(_) => {
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Authentication service unavailable",
                )
                    .into_response();
            }
        }
    } else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Not authenticated. Provide a valid session cookie or bot bearer token.",
        )
            .into_response();
    };

    if !rate_limiters.admit_authenticated_ws(actor.credential_id().as_str()) {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Too many authenticated reconnects. Please try again later.",
        )
            .into_response();
    }

    let engine = state.engine.clone();
    let auth = state.auth.clone();
    let shutdown = state.shutdown.clone();
    ws.max_message_size(64 * 1024) // 64 KB max WS message
        .on_upgrade(move |socket| {
            handle_ws_connection(socket, engine, auth, actor, nickname, avatar_url, shutdown)
        })
        .into_response()
}

async fn handle_ws_connection(
    socket: WebSocket,
    engine: Arc<ChatEngine>,
    auth: AuthService,
    actor: Actor,
    nickname: String,
    avatar_url: Option<String>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let credential_lease = match auth.register_live(&actor).await {
        Ok(lease) => lease,
        Err(_) => return,
    };
    let (session_id, mut event_rx) = match engine.connect(
        Some(actor.user_id().as_str().to_owned()),
        nickname.clone(),
        Protocol::WebSocket,
        avatar_url,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            warn!(%nickname, error = %e, "WebSocket connection rejected");
            return;
        }
    };
    if engine
        .bind_authenticated_actor(session_id, actor.clone())
        .is_err()
    {
        engine.disconnect(session_id);
        return;
    }
    if engine.send_own_presence(session_id).await.is_err() {
        engine.disconnect(session_id);
        return;
    }
    let Some(session) = engine.get_session(session_id) else {
        return;
    };
    let overflow_cancel = session.overflow_cancellation_token();

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (heartbeat_tx, mut heartbeat_rx) = tokio::sync::mpsc::channel::<()>(1);

    let writer_auth = auth.clone();
    let writer_actor = actor.clone();
    let writer_cancel = credential_lease.cancellation_token();
    let writer_shutdown = shutdown.clone();
    let writer_engine = engine.clone();
    let writer_session = session.clone();
    let writer_overflow = overflow_cancel.clone();
    let writer_done = tokio_util::sync::CancellationToken::new();
    let writer_finished = writer_done.clone();
    let write_handle = tokio::spawn(async move {
        enum WriterAction {
            Event(Box<ChatEvent>),
            Ping,
            PongReceived,
        }

        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        heartbeat.tick().await;
        let mut awaiting_pong = false;
        loop {
            let action = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                _ = writer_shutdown.cancelled() => break,
                _ = writer_overflow.cancelled() => break,
                _ = crate::auth::authority::wait_for_expiry(writer_actor.expires_at()) => break,
                _ = heartbeat.tick() => WriterAction::Ping,
                pong = heartbeat_rx.recv() => match pong {
                    Some(()) => WriterAction::PongReceived,
                    None => break,
                },
                event = event_rx.recv() => match event {
                    Some(event) => WriterAction::Event(Box::new(event)),
                    None => break,
                },
            };
            if matches!(action, WriterAction::PongReceived) {
                awaiting_pong = false;
                continue;
            }
            if matches!(action, WriterAction::Ping) && awaiting_pong {
                break;
            }
            if writer_auth.validate_actor(&writer_actor).await.is_err() {
                break;
            }
            let message = match action {
                WriterAction::Event(event) => {
                    if let Some(guard) = writer_session.take_delivery_guard() {
                        let authorized = writer_engine
                            .delivery_guard_is_current(&writer_actor, &guard)
                            .await;
                        if !authorized {
                            warn!(
                                guard = guard.kind(),
                                "WebSocket delivery guard denied queued event"
                            );
                            if matches!(
                                guard,
                                crate::engine::user_session::DeliveryGuard::ServerPermissions(_)
                            ) {
                                continue;
                            }
                            break;
                        }
                    }
                    match serde_json::to_string(&event) {
                        Ok(json) => Message::Text(json.into()),
                        Err(e) => {
                            error!(error = %e, "failed to serialize event");
                            continue;
                        }
                    }
                }
                WriterAction::Ping => {
                    awaiting_pong = true;
                    Message::Ping(Vec::new().into())
                }
                WriterAction::PongReceived => unreachable!(),
            };
            let sent = tokio::select! {
                _ = writer_cancel.cancelled() => false,
                _ = writer_shutdown.cancelled() => false,
                _ = writer_overflow.cancelled() => false,
                result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ws_sender.send(message),
                ) => matches!(result, Ok(Ok(()))),
            };
            if !sent {
                break;
            }
        }
        writer_finished.cancel();
    });

    let engine_ref = engine.clone();
    let mut ws_mutation_count: u32 = 0;
    let mut ws_mutation_window_start = Instant::now();
    let mut ws_read_count: u32 = 0;
    let mut ws_read_window_start = Instant::now();
    const WS_COMMANDS_PER_SECOND: u32 = 30;
    const WS_READS_PER_SECOND: u32 = 120;

    loop {
        let msg = match tokio::select! {
            _ = credential_lease.cancelled() => break,
            _ = shutdown.cancelled() => break,
            _ = crate::auth::authority::wait_for_expiry(actor.expires_at()) => break,
            _ = writer_done.cancelled() => break,
            _ = overflow_cancel.cancelled() => break,
            result = ws_receiver.next() => result,
        } {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => {
                warn!(error = %e, "WebSocket read error");
                break;
            }
            None => break, // Stream closed
        };

        match msg {
            Message::Text(text) => {
                if auth.validate_actor(&actor).await.is_err() {
                    break;
                }
                let is_read = websocket_command_is_read(&text);
                let admitted = if is_read {
                    fixed_window_admit(
                        &mut ws_read_count,
                        &mut ws_read_window_start,
                        WS_READS_PER_SECOND,
                    )
                } else {
                    fixed_window_admit(
                        &mut ws_mutation_count,
                        &mut ws_mutation_window_start,
                        WS_COMMANDS_PER_SECOND,
                    )
                };
                if !admitted {
                    if let Some(session) = engine_ref.get_session(session_id) {
                        let message = if is_read {
                            "Rate limited: too many read commands; retry shortly"
                        } else {
                            "Rate limited: too many mutation commands; retry shortly"
                        };
                        if let Some(request_id) = websocket_command_correlation(&text) {
                            let _ = session.send(ChatEvent::CommandError {
                                request_id,
                                code: "RATE_LIMITED".into(),
                                message: message.into(),
                                retryable: true,
                            });
                        } else {
                            let _ = session.send(ChatEvent::Error {
                                code: "RATE_LIMITED".into(),
                                message: message.into(),
                            });
                        }
                    }
                    continue;
                }
                handle_client_message(&engine_ref, session_id, &text).await;
            }
            Message::Pong(_) => {
                let _ = heartbeat_tx.try_send(());
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    engine.disconnect(session_id);
    write_handle.abort();
    info!(%session_id, %nickname, "WebSocket connection closed");
}

fn fixed_window_admit(count: &mut u32, window_start: &mut Instant, limit: u32) -> bool {
    if window_start.elapsed() >= std::time::Duration::from_secs(1) {
        *count = 0;
        *window_start = Instant::now();
    }
    *count = count.saturating_add(1);
    let admitted = *count <= limit;
    crate::runtime_metrics::record(
        crate::runtime_metrics::Operation::CommandAdmission,
        admitted,
        std::time::Duration::ZERO,
    );
    admitted
}

fn websocket_command_correlation(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("request_id")
                .or_else(|| value.get("nonce"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 128)
                .map(str::to_owned)
        })
}

/// Read-only bootstrap and query commands have an independent admission budget
/// so reconnect hydration cannot consume the budget needed for user mutations.
fn websocket_command_is_read(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    matches!(
        value.get("type").and_then(serde_json::Value::as_str),
        Some(
            "sync"
                | "fetch_history"
                | "list_channels"
                | "get_members"
                | "list_servers"
                | "list_direct_conversations"
                | "get_unread_counts"
                | "list_roles"
                | "list_channel_permission_overrides"
                | "list_categories"
                | "get_presences"
                | "search_messages"
                | "get_notification_settings"
                | "get_user_profile"
                | "get_pinned_messages"
                | "list_threads"
                | "list_forum_tags"
                | "get_thread_tags"
                | "list_bookmarks"
                | "list_bans"
                | "get_audit_log"
                | "list_automod_rules"
                | "list_invites"
                | "list_events"
                | "list_rsvps"
                | "get_community_settings"
                | "discover_servers"
                | "list_channel_follows"
                | "list_templates"
                | "list_webhooks"
                | "list_owned_bots"
                | "list_bot_tokens"
                | "list_slash_commands"
                | "list_o_auth2_apps"
                | "get_server_limits"
                | "get_bluesky_identity"
                | "get_atproto_sync_setting"
        )
    )
}

async fn handle_client_message(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    text: &str,
) {
    let correlation_id = websocket_command_correlation(text);
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                category = ?e.classify(),
                line = e.line(),
                column = e.column(),
                "invalid client message"
            );
            if let Some(request_id) = correlation_id
                && let Some(session) = engine.get_session(session_id)
            {
                let _ = session.send(ChatEvent::CommandError {
                    request_id,
                    code: "INVALID_INPUT".into(),
                    message: "invalid client message".into(),
                    retryable: false,
                });
            }
            return;
        }
    };

    let (msg, lifecycle_success_id) = match msg {
        ClientMessage::LifecycleCommand {
            request_id,
            command,
        } if lifecycle_command_allowed(&command) => (*command, Some(request_id)),
        ClientMessage::LifecycleCommand { request_id, .. } => {
            if let Some(session) = engine.get_session(session_id) {
                let _ = session.send(ChatEvent::CommandError {
                    request_id,
                    code: "INVALID_INPUT".into(),
                    message: "command is not a lifecycle mutation".into(),
                    retryable: false,
                });
            }
            return;
        }
        message => (message, None),
    };

    let result = match msg {
        ClientMessage::LifecycleCommand { .. } => unreachable!("lifecycle envelope was unwrapped"),
        ClientMessage::Sync {
            request_id,
            protocol_version,
            subscriptions,
            cursor,
            limit,
        } => {
            if protocol_version != 2 {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::ResyncRequired {
                        request_id,
                        reason: crate::engine::replay::ResyncReason::ProtocolChanged,
                    });
                }
                Ok(())
            } else {
                match engine
                    .synchronize(
                        session_id,
                        &subscriptions,
                        cursor.as_deref(),
                        limit.unwrap_or(100),
                    )
                    .await
                {
                    Ok(event) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let event = match event {
                                crate::engine::chat_engine::Synchronization::Snapshot(snapshot) => {
                                    ChatEvent::SyncSnapshot {
                                        request_id,
                                        snapshot,
                                    }
                                }
                                crate::engine::chat_engine::Synchronization::Replay(batch) => {
                                    ChatEvent::ReplayBatch { request_id, batch }
                                }
                            };
                            let _ = session.send_guarded(
                                event,
                                Some(crate::engine::user_session::DeliveryGuard::Conversations(
                                    subscriptions,
                                )),
                            );
                        }
                        Ok(())
                    }
                    Err(crate::engine::replay::ReplayError::ResyncRequired(reason)) => {
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send(ChatEvent::ResyncRequired { request_id, reason });
                        }
                        Ok(())
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
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
            let fallback = nonce
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let request_id = request_id.as_deref().unwrap_or(&fallback);
            let client_message_id = client_message_id.as_deref().unwrap_or(&fallback);
            engine
                .submit_channel_message(
                    session_id,
                    crate::engine::messaging::SendMessageCommand {
                        request_id,
                        client_message_id,
                        operation_generation: Some(&operation_generation),
                        conversation_id: conversation_id.as_deref(),
                        server_id: &server_id,
                        channel: &channel,
                        content: &content,
                        content_format,
                        reply_to_id: reply_to.as_deref(),
                        attachment_ids: attachment_ids.as_deref().unwrap_or(&[]),
                        mentions: &mentions,
                    },
                    nonce.as_deref(),
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
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
            let fallback = nonce
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            engine
                .submit_direct_message(
                    session_id,
                    crate::engine::messaging::SendDirectMessageCommand {
                        request_id: request_id.as_deref().unwrap_or(&fallback),
                        client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                        operation_generation: Some(&operation_generation),
                        recipient: &recipient,
                        content: &content,
                        content_format,
                        reply_to_id: reply_to.as_deref(),
                        attachment_ids: attachment_ids.as_deref().unwrap_or(&[]),
                    },
                    nonce.as_deref(),
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
        }
        ClientMessage::ListDirectConversations => {
            engine.list_direct_conversations(session_id).await
        }
        ClientMessage::JoinChannel { server_id, channel } => {
            engine.join_channel(session_id, &server_id, &channel).await
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
        } => {
            engine
                .set_topic(session_id, &server_id, &channel, topic)
                .await
        }
        ClientMessage::FetchHistory {
            server_id,
            channel,
            before,
            limit,
        } => {
            if let Some(actor) = engine.get_authenticated_actor(session_id) {
                let limit = limit.unwrap_or(50).clamp(1, 200);
                match engine
                    .fetch_history(&server_id, &channel, before.as_deref(), limit, &actor)
                    .await
                {
                    Ok((messages, has_more, stamp)) => {
                        if !engine.authorization_stamp_is_current(&actor, &stamp).await {
                            return;
                        }
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send_guarded(
                                ChatEvent::History {
                                    server_id,
                                    channel,
                                    messages,
                                    has_more,
                                },
                                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                    stamp,
                                ])),
                            );
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            } else {
                Err("resource unavailable".into())
            }
        }
        ClientMessage::ListChannels { server_id } => {
            if let Some(actor) = engine.get_authenticated_actor(session_id) {
                match engine
                    .list_visible_channels_for_actor(&server_id, &actor)
                    .await
                {
                    Ok((channels, stamp)) => {
                        if !engine.authorization_stamp_is_current(&actor, &stamp).await {
                            return;
                        }
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send_guarded(
                                ChatEvent::ChannelList {
                                    server_id,
                                    channels,
                                },
                                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                    stamp,
                                ])),
                            );
                        }
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            } else {
                Err("resource unavailable".into())
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
                let Some(actor) = engine.get_authenticated_actor(session_id) else {
                    return;
                };
                match engine
                    .get_visible_members(&actor, &server_id, &channel)
                    .await
                {
                    Ok((member_infos, stamp)) => {
                        if !engine.authorization_stamp_is_current(&actor, &stamp).await {
                            return;
                        }
                        if let Some(session) = engine.get_session(session_id) {
                            let _ = session.send_guarded(
                                ChatEvent::Names {
                                    server_id,
                                    channel,
                                    members: member_infos,
                                },
                                Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                    stamp,
                                ])),
                            );
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
            match engine.get_authenticated_actor(session_id) {
                Some(actor) => match engine.create_server_for_actor(&actor, name, icon_url).await {
                    Ok(_server_id) => {
                        if let Some(session) = engine.get_session(session_id)
                            && let Some(ref uid) = session.user_id
                        {
                            let servers = engine.list_servers_for_user(uid).await;
                            let _ = session.send(ChatEvent::ServerList { servers });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                },
                None => Err("UNAUTHENTICATED: authentication required".into()),
            }
        }
        ClientMessage::JoinServer { server_id } => {
            match engine.get_authenticated_actor(session_id) {
                Some(actor) => match engine.join_server_for_actor(&actor, &server_id).await {
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
                },
                None => Err("UNAUTHENTICATED: authentication required".into()),
            }
        }
        ClientMessage::LeaveServer { server_id } => {
            match engine.get_authenticated_actor(session_id) {
                Some(actor) => match engine.leave_server_for_actor(&actor, &server_id).await {
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
                },
                None => Err("UNAUTHENTICATED: authentication required".into()),
            }
        }
        ClientMessage::CreateChannel {
            server_id,
            name,
            category_id,
            is_private,
            channel_type,
        } => {
            match engine
                .create_channel_in_server(
                    session_id,
                    &server_id,
                    &name,
                    category_id.as_deref(),
                    is_private.unwrap_or(false),
                    channel_type.as_deref().unwrap_or("text"),
                )
                .await
            {
                Ok(_) => {
                    engine
                        .send_visible_channel_list(session_id, server_id)
                        .await
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteChannel { server_id, channel } => {
            match engine
                .delete_channel_in_server(session_id, &server_id, &channel)
                .await
            {
                Ok(()) => {
                    engine
                        .send_visible_channel_list(session_id, server_id)
                        .await
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteServer { server_id } => {
            match engine.get_authenticated_actor(session_id) {
                Some(actor) => match engine.delete_owned_server(&server_id, &actor).await {
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
                },
                None => Err("UNAUTHENTICATED: authentication required".into()),
            }
        }
        ClientMessage::UpdateServer {
            server_id,
            name,
            icon_url,
        } => {
            match engine.get_authenticated_actor(session_id) {
                Some(actor) => {
                    match engine
                        .update_server_settings_for_actor(
                            &actor,
                            &server_id,
                            name.as_deref(),
                            icon_url.as_deref(),
                        )
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
                None => Err("UNAUTHENTICATED: authentication required".into()),
            }
        }
        ClientMessage::UpdateMemberRole {
            server_id,
            user_id,
            role,
        } => match engine.get_authenticated_actor(session_id) {
            Some(actor) => {
                engine
                    .update_member_role_for_actor(&actor, &server_id, &user_id, &role)
                    .await
            }
            None => Err("UNAUTHENTICATED: authentication required".into()),
        },
        ClientMessage::EditMessage {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
            content,
            content_format,
            mentions,
        } => {
            let fallback = uuid::Uuid::new_v4().to_string();
            engine
                .submit_edit_message(
                    session_id,
                    crate::engine::messaging::EditMessageCommand {
                        request_id: request_id.as_deref().unwrap_or(&fallback),
                        client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                        operation_generation: Some(&operation_generation),
                        message_id: &message_id,
                        content: &content,
                        content_format,
                        mentions: &mentions,
                    },
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
        }
        ClientMessage::DeleteMessage {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
        } => {
            let fallback = uuid::Uuid::new_v4().to_string();
            engine
                .submit_delete_message(
                    session_id,
                    crate::engine::messaging::EntityCommand {
                        request_id: request_id.as_deref().unwrap_or(&fallback),
                        client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                        operation_generation: Some(&operation_generation),
                        message_id: &message_id,
                    },
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
        }
        ClientMessage::AddReaction {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
            emoji,
        } => {
            let fallback = uuid::Uuid::new_v4().to_string();
            engine
                .submit_reaction(
                    session_id,
                    crate::engine::messaging::ReactionCommand {
                        request_id: request_id.as_deref().unwrap_or(&fallback),
                        client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                        operation_generation: Some(&operation_generation),
                        message_id: &message_id,
                        emoji: &emoji,
                    },
                    true,
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
        }
        ClientMessage::RemoveReaction {
            operation_generation,
            request_id,
            client_message_id,
            message_id,
            emoji,
        } => {
            let fallback = uuid::Uuid::new_v4().to_string();
            engine
                .submit_reaction(
                    session_id,
                    crate::engine::messaging::ReactionCommand {
                        request_id: request_id.as_deref().unwrap_or(&fallback),
                        client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                        operation_generation: Some(&operation_generation),
                        message_id: &message_id,
                        emoji: &emoji,
                    },
                    false,
                )
                .await
                .map(|_| ())
                .map_err(|error| format!("{}: {}", error.code(), error.safe_message()))
        }
        ClientMessage::Typing { server_id, channel } => {
            engine.send_typing(session_id, &server_id, &channel)
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
            let fallback = uuid::Uuid::new_v4().to_string();
            let conversation_id = match conversation_id {
                Some(id) => Ok(id),
                None => {
                    engine
                        .conversation_id_for_channel(&server_id, &channel)
                        .await
                }
            };
            match conversation_id {
                Ok(conversation_id) => engine
                    .submit_mark_read(
                        session_id,
                        crate::engine::messaging::ReadCommand {
                            request_id: request_id.as_deref().unwrap_or(&fallback),
                            client_message_id: client_message_id.as_deref().unwrap_or(&fallback),
                            operation_generation: Some(&operation_generation),
                            conversation_id: &conversation_id,
                            message_id: &message_id,
                        },
                    )
                    .await
                    .map(|_| ())
                    .map_err(|error| format!("{}: {}", error.code(), error.safe_message())),
                Err(error) => Err(error),
            }
        }
        ClientMessage::GetUnreadCounts { server_id } => {
            match engine.get_unread_counts(session_id, &server_id).await {
                Ok((counts, stamps)) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send_guarded(
                            ChatEvent::UnreadCounts { server_id, counts },
                            Some(crate::engine::user_session::DeliveryGuard::Stamps(stamps)),
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        // ── Roles ──
        ClientMessage::ListRoles { server_id } => {
            match engine.list_roles(session_id, &server_id).await {
                Ok((version, roles, member_roles)) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::RoleList {
                            server_id,
                            version,
                            roles,
                            member_roles: Some(member_roles),
                        });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
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
                        .create_role(session_id, &server_id, &name, color.as_deref(), perms)
                        .await
                    {
                        Ok(_) => {
                            engine
                                .broadcast_role_snapshot(session_id, &server_id, None)
                                .await
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
                                    session_id,
                                    &server_id,
                                    &role_id,
                                    &name,
                                    color.as_deref(),
                                    permissions,
                                )
                                .await
                            {
                                Ok(_) => {
                                    engine
                                        .broadcast_role_snapshot(session_id, &server_id, None)
                                        .await
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
                        Ok(()) => {
                            match engine.delete_role(session_id, &server_id, &role_id).await {
                                Ok(()) => {
                                    engine
                                        .broadcast_role_snapshot(session_id, &server_id, None)
                                        .await
                                }
                                Err(e) => Err(e),
                            }
                        }
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
                Ok(_actor_uid) => match engine
                    .assign_role(session_id, &server_id, &user_id, &role_id)
                    .await
                {
                    Ok(_) => {
                        engine
                            .broadcast_role_snapshot(session_id, &server_id, Some(&user_id))
                            .await
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
                Ok(_actor_uid) => match engine
                    .remove_role(session_id, &server_id, &user_id, &role_id)
                    .await
                {
                    Ok(_) => {
                        engine
                            .broadcast_role_snapshot(session_id, &server_id, Some(&user_id))
                            .await
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        ClientMessage::ListChannelPermissionOverrides {
            server_id,
            channel_id,
        } => match engine
            .list_channel_permission_overrides(session_id, &server_id, &channel_id)
            .await
        {
            Ok(overrides) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send_guarded(
                        ChatEvent::ChannelPermissionOverrideList {
                            server_id: server_id.clone(),
                            channel_id,
                            overrides,
                        },
                        Some(
                            crate::engine::user_session::DeliveryGuard::ServerPermissions(vec![(
                                server_id,
                                Permissions::MANAGE_CHANNELS,
                            )]),
                        ),
                    );
                }
                Ok(())
            }
            Err(error) => Err(error),
        },
        ClientMessage::SetChannelPermissionOverride {
            server_id,
            channel_id,
            target_type,
            target_id,
            allow_bits,
            deny_bits,
        } => {
            match (
                crate::engine::ids::ServerId::from_stored(server_id.clone()),
                crate::engine::ids::ChannelId::from_stored(channel_id.clone()),
            ) {
                (Ok(server_resource_id), Ok(channel_resource_id)) => match engine
                    .set_channel_permission_override(
                        session_id,
                        crate::engine::organization::ChannelOverrideUpdate {
                            server_id: &server_resource_id,
                            channel_id: &channel_resource_id,
                            target_type: &target_type,
                            target_id: &target_id,
                            allow_bits,
                            deny_bits,
                        },
                    )
                    .await
                {
                    Ok(()) => {
                        engine
                            .broadcast_channel_permission_overrides(
                                session_id,
                                &server_id,
                                &channel_id,
                            )
                            .await
                    }
                    Err(error) => Err(error),
                },
                _ => Err("INVALID_INPUT: invalid resource id".to_owned()),
            }
        }
        ClientMessage::DeleteChannelPermissionOverride {
            server_id,
            channel_id,
            target_type,
            target_id,
        } => match engine
            .delete_channel_permission_override(
                session_id,
                &server_id,
                &channel_id,
                &target_type,
                &target_id,
            )
            .await
        {
            Ok(()) => {
                engine
                    .broadcast_channel_permission_overrides(session_id, &server_id, &channel_id)
                    .await
            }
            Err(error) => Err(error),
        },
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
                Ok(_) => match engine.create_category(session_id, &server_id, &name).await {
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
                Ok(_) => match engine
                    .update_category(session_id, &server_id, &category_id, &name)
                    .await
                {
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
                Ok(_) => match engine
                    .delete_category(session_id, &server_id, &category_id)
                    .await
                {
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
                Ok(_) => match engine
                    .reorder_channels(session_id, &server_id, &channels)
                    .await
                {
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
            match engine.get_server_presences(session_id, &server_id).await {
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
            request_id,
            server_id,
            query,
            channel,
            limit,
            offset,
            continuation,
        } => {
            let limit = limit.unwrap_or(25).min(50);
            let offset = offset.unwrap_or(0);
            let Some(actor) = engine.get_authenticated_actor(session_id) else {
                return;
            };
            match engine
                .search_messages(
                    &actor,
                    crate::engine::chat_engine::SearchMessagesRequest {
                        server_id: &server_id,
                        query: &query,
                        channel_name: channel.as_deref(),
                        limit,
                        offset,
                        continuation: continuation.as_deref(),
                    },
                )
                .await
            {
                Ok(page) => {
                    if !engine
                        .authorization_stamp_is_current(&actor, &page.stamp)
                        .await
                    {
                        return;
                    }
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send_guarded(
                            ChatEvent::SearchResults {
                                request_id,
                                server_id,
                                query,
                                results: page.results,
                                total_count: page.total_count,
                                offset: page.offset,
                                next_continuation: page.next_continuation,
                                restarted: page.restarted,
                            },
                            Some(crate::engine::user_session::DeliveryGuard::Stamps(vec![
                                page.stamp,
                            ])),
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
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
            let Some(actor) = engine.get_authenticated_actor(session_id) else {
                return;
            };
            match engine.get_user_profile(&actor, &user_id).await {
                Ok((profile, stamp)) => {
                    let current = match &stamp {
                        Some(stamp) => engine.authorization_stamp_is_current(&actor, stamp).await,
                        None => engine.actor_is_current(&actor).await,
                    };
                    if !current {
                        return;
                    }
                    if let Some(session) = engine.get_session(session_id) {
                        let guard = match stamp {
                            Some(stamp) => {
                                crate::engine::user_session::DeliveryGuard::Stamps(vec![stamp])
                            }
                            None => crate::engine::user_session::DeliveryGuard::ActorCurrent,
                        };
                        let _ =
                            session.send_guarded(ChatEvent::UserProfile { profile }, Some(guard));
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
        ClientMessage::UnarchiveThread {
            server_id,
            thread_id,
        } => {
            engine
                .unarchive_thread(session_id, &server_id, &thread_id)
                .await
        }
        ClientMessage::ListThreads { server_id, channel } => {
            engine.list_threads(session_id, &server_id, &channel).await
        }
        ClientMessage::CreateForumTag {
            server_id,
            channel,
            name,
            emoji,
            moderated,
        } => {
            engine
                .create_forum_tag(
                    session_id,
                    &server_id,
                    &channel,
                    &name,
                    emoji.as_deref(),
                    moderated,
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
            engine
                .update_forum_tag(
                    session_id,
                    &server_id,
                    &channel,
                    &tag_id,
                    &name,
                    emoji.as_deref(),
                    moderated,
                    position,
                )
                .await
        }
        ClientMessage::DeleteForumTag {
            server_id,
            channel,
            tag_id,
        } => {
            engine
                .delete_forum_tag(session_id, &server_id, &channel, &tag_id)
                .await
        }
        ClientMessage::ListForumTags { server_id, channel } => {
            engine
                .list_forum_tags(session_id, &server_id, &channel)
                .await
        }
        ClientMessage::SetThreadTags {
            server_id,
            thread_id,
            tag_ids,
        } => {
            engine
                .set_thread_tags(session_id, &server_id, &thread_id, tag_ids)
                .await
        }
        ClientMessage::GetThreadTags {
            server_id,
            thread_id,
        } => {
            engine
                .get_thread_tags(session_id, &server_id, &thread_id)
                .await
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
            engine
                .create_automod_rule(
                    session_id,
                    &crate::engine::chat_engine::CreateAutomodRuleRequest {
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
                    &crate::engine::chat_engine::UpdateAutomodRuleRequest {
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
                    &crate::engine::chat_engine::CreateServerEventRequest {
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
        ClientMessage::PublishAnnouncement { message_id } => {
            engine.publish_announcement(session_id, &message_id).await
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
        ClientMessage::InstantiateTemplate {
            template_id,
            server_name,
        } => {
            engine
                .instantiate_template(session_id, &template_id, &server_name)
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
        ClientMessage::ListOwnedBots => engine.list_owned_bots(session_id).await,
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
            request_id,
            server_id,
            channel,
            command_name,
            args_json,
        } => {
            let result = engine
                .invoke_slash_command(
                    session_id,
                    &server_id,
                    &channel,
                    &command_name,
                    args_json.as_deref(),
                )
                .await;
            if result.is_ok()
                && let Some(session) = engine.get_session(session_id)
            {
                let _ = session.send(ChatEvent::InteractionInvoked { request_id });
            }
            result
        }
        ClientMessage::InvokeMessageComponent {
            request_id,
            message_id,
            custom_id,
            values,
        } => {
            let result = engine
                .invoke_message_component(session_id, &message_id, &custom_id, &values)
                .await;
            if result.is_ok()
                && let Some(session) = engine.get_session(session_id)
            {
                let _ = session.send(ChatEvent::InteractionInvoked { request_id });
            }
            result
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
            client_type,
        } => {
            engine
                .create_oauth2_app(
                    session_id,
                    &name,
                    description.as_deref(),
                    &redirect_uris,
                    &client_type,
                )
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
        } => match engine.get_authenticated_actor(session_id) {
            Some(actor) => {
                engine
                    .set_member_avatar_for_actor(&actor, &server_id, avatar_url.as_deref())
                    .await
            }
            None => Err("UNAUTHENTICATED: authentication required".into()),
        },

        ClientMessage::SetVanityCode {
            server_id,
            vanity_code,
        } => {
            engine
                .set_vanity_code(session_id, &server_id, vanity_code.as_deref())
                .await
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

    match result {
        Ok(()) => {
            if let Some(request_id) = lifecycle_success_id
                && let Some(session) = engine.get_session(session_id)
            {
                let _ = session.send(ChatEvent::LifecycleCommandSucceeded { request_id });
            }
        }
        Err(error) => {
            let (code, message) = split_safe_error(&error);
            if let Some(request_id) = correlation_id {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::CommandError {
                        request_id,
                        code: code.to_owned(),
                        message: message.to_owned(),
                        retryable: code == "DEPENDENCY_UNAVAILABLE",
                    });
                }
            } else {
                send_error(engine, session_id, code, message);
            }
        }
    }
}

fn lifecycle_command_allowed(command: &ClientMessage) -> bool {
    !matches!(
        command,
        ClientMessage::LifecycleCommand { .. }
            | ClientMessage::Sync { .. }
            | ClientMessage::SendMessage { .. }
            | ClientMessage::SendDirectMessage { .. }
            | ClientMessage::EditMessage { .. }
            | ClientMessage::DeleteMessage { .. }
            | ClientMessage::AddReaction { .. }
            | ClientMessage::RemoveReaction { .. }
            | ClientMessage::MarkRead { .. }
            | ClientMessage::InvokeSlashCommand { .. }
            | ClientMessage::InvokeMessageComponent { .. }
    )
}

fn split_safe_error(error: &str) -> (&str, &str) {
    error
        .split_once(": ")
        .filter(|(code, _)| {
            code.chars()
                .all(|character| character.is_ascii_uppercase() || character == '_')
        })
        .unwrap_or(("COMMAND_FAILED", error))
}

fn send_error(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
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
    use tokio::sync::mpsc;

    #[test]
    fn bootstrap_reads_do_not_consume_mutation_admission() {
        let before = crate::runtime_metrics::snapshot();
        let admission_index = crate::runtime_metrics::Operation::CommandAdmission as usize;
        let mut read_count = 0;
        let mut read_window = Instant::now();
        let mut mutation_count = 0;
        let mut mutation_window = Instant::now();

        for _ in 0..120 {
            assert!(fixed_window_admit(&mut read_count, &mut read_window, 120));
        }
        assert!(!fixed_window_admit(&mut read_count, &mut read_window, 120));
        assert!(fixed_window_admit(
            &mut mutation_count,
            &mut mutation_window,
            30
        ));
        let after = crate::runtime_metrics::snapshot();
        assert!(after.succeeded[admission_index] >= before.succeeded[admission_index] + 121);
        assert!(after.failed[admission_index] > before.failed[admission_index]);
    }

    #[test]
    fn websocket_admission_classifies_reads_and_preserves_correlation() {
        assert!(websocket_command_is_read(
            r#"{"type":"list_channels","server_id":"server"}"#
        ));
        assert!(websocket_command_is_read(
            r#"{"type":"sync","request_id":"sync-1","protocol_version":2,"subscriptions":[]}"#
        ));
        assert!(websocket_command_is_read(r#"{"type":"list_owned_bots"}"#));
        assert!(!websocket_command_is_read(
            r#"{"type":"set_presence","status":"idle"}"#
        ));
        assert!(!websocket_command_is_read("not json"));
        assert_eq!(
            websocket_command_correlation(
                r#"{"type":"sync","request_id":"sync-1","protocol_version":2,"subscriptions":[]}"#
            )
            .as_deref(),
            Some("sync-1")
        );
    }

    /// Helper to deserialize a JSON string into a ClientMessage.
    fn parse_msg(json: &str) -> Result<ClientMessage, serde_json::Error> {
        serde_json::from_str(json)
    }

    async fn forum_wire_fixture(
        owner: bool,
    ) -> (
        ChatEngine,
        sqlx::SqlitePool,
        crate::auth::authority::AuthService,
        crate::auth::authority::CredentialId,
        crate::engine::events::ConnectionId,
        mpsc::Receiver<ChatEvent>,
    ) {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES \
             ('server','owner','owner'),('server','member','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO roles(id,server_id,name,permissions,is_default) \
             VALUES('everyone','server','@everyone',?,1)",
        )
        .bind(crate::engine::permissions::DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,server_id,name,channel_type) \
             VALUES('forum','server','#forum','forum')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let user_id = if owner { "owner" } else { "member" };
        let auth = crate::auth::authority::AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session(user_id).await.unwrap();
        let credential_id = actor.credential_id().clone();
        let engine = ChatEngine::new(pool.clone(), auth.clone(), "replay-secret", 4000, 100);
        engine.load_servers_from_db().await.unwrap();
        engine.load_channels_from_db().await.unwrap();
        let (session_id, receiver) = engine
            .connect(
                Some(user_id.into()),
                user_id.into(),
                Protocol::WebSocket,
                None,
            )
            .unwrap();
        engine.bind_authenticated_actor(session_id, actor).unwrap();
        (engine, pool, auth, credential_id, session_id, receiver)
    }

    async fn receive_command_error(
        receiver: &mut mpsc::Receiver<ChatEvent>,
    ) -> (String, String, bool) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
                .await
                .expect("command response timed out")
                .expect("command response channel closed")
            {
                ChatEvent::CommandError {
                    code,
                    message,
                    retryable,
                    ..
                } => return (code, message, retryable),
                _ => continue,
            }
        }
    }

    async fn receive_lifecycle_success(receiver: &mut mpsc::Receiver<ChatEvent>) -> String {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
                .await
                .expect("command response timed out")
                .expect("command response channel closed")
            {
                ChatEvent::LifecycleCommandSucceeded { request_id } => return request_id,
                _ => continue,
            }
        }
    }

    #[tokio::test]
    async fn lifecycle_mutation_reports_success_only_after_command_acceptance() {
        let (engine, pool, _, _, session_id, mut receiver) = forum_wire_fixture(true).await;
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"lifecycle_command","request_id":"create-tag","command":{"type":"create_forum_tag","server_id":"server","channel":"#forum","name":"accepted","emoji":null,"moderated":false}}"##,
        )
        .await;

        assert_eq!(receive_lifecycle_success(&mut receiver).await, "create-tag");
        let persisted: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM forum_tags WHERE name='accepted'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(persisted, 1);
    }

    #[tokio::test]
    async fn forum_commands_preserve_validation_denial_auth_and_dependency_fault_classes() {
        let (engine, pool, auth, credential_id, session_id, mut receiver) =
            forum_wire_fixture(true).await;
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"create_forum_tag","request_id":"invalid","server_id":"server","channel":"#forum","name":"","emoji":null,"moderated":false}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "INVALID_INPUT".into(),
                "forum tag name must contain 1 to 100 bytes".into(),
                false,
            )
        );

        sqlx::query(
            "CREATE TRIGGER reject_forum_tag BEFORE INSERT ON forum_tags \
             BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"create_forum_tag","request_id":"dependency","server_id":"server","channel":"#forum","name":"tag","emoji":null,"moderated":false}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "DEPENDENCY_UNAVAILABLE".into(),
                "dependency unavailable".into(),
                true,
            )
        );
        sqlx::query("DROP TRIGGER reject_forum_tag")
            .execute(&pool)
            .await
            .unwrap();
        auth.revoke_credential(&credential_id).await.unwrap();
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"create_forum_tag","request_id":"auth","server_id":"server","channel":"#forum","name":"tag","emoji":null,"moderated":false}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "UNAUTHENTICATED".into(),
                "authentication required".into(),
                false,
            )
        );

        let (engine, _, _, _, session_id, mut receiver) = forum_wire_fixture(false).await;
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"create_forum_tag","request_id":"denied","server_id":"server","channel":"#forum","name":"tag","emoji":null,"moderated":false}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "RESOURCE_UNAVAILABLE".into(),
                "resource unavailable".into(),
                false,
            )
        );
    }

    #[tokio::test]
    async fn moderation_commands_preserve_validation_denial_auth_and_dependency_fault_classes() {
        let (engine, pool, auth, credential_id, session_id, mut receiver) =
            forum_wire_fixture(true).await;
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"ban_member","request_id":"invalid","server_id":"server","user_id":"member","delete_message_days":8}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "INVALID_INPUT".into(),
                "delete_message_days must be between 0 and 7".into(),
                false,
            )
        );

        sqlx::query(
            "CREATE TRIGGER reject_timeout_audit BEFORE INSERT ON audit_log \
             WHEN NEW.action_type='member_timeout' BEGIN SELECT RAISE(ABORT,'forced'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let timeout_until = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        let dependency = serde_json::json!({
            "type": "timeout_member",
            "request_id": "dependency",
            "server_id": "server",
            "user_id": "member",
            "timeout_until": timeout_until,
        })
        .to_string();
        handle_client_message(&engine, session_id, &dependency).await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "DEPENDENCY_UNAVAILABLE".into(),
                "dependency unavailable".into(),
                true,
            )
        );
        sqlx::query("DROP TRIGGER reject_timeout_audit")
            .execute(&pool)
            .await
            .unwrap();
        auth.revoke_credential(&credential_id).await.unwrap();
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"kick_member","request_id":"auth","server_id":"server","user_id":"member"}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "UNAUTHENTICATED".into(),
                "authentication required".into(),
                false,
            )
        );

        let (engine, _, _, _, session_id, mut receiver) = forum_wire_fixture(false).await;
        handle_client_message(
            &engine,
            session_id,
            r##"{"type":"ban_member","request_id":"denied","server_id":"server","user_id":"owner","delete_message_days":0}"##,
        )
        .await;
        assert_eq!(
            receive_command_error(&mut receiver).await,
            (
                "RESOURCE_UNAVAILABLE".into(),
                "resource unavailable".into(),
                false,
            )
        );
    }

    // ── Core messaging ──

    #[test]
    fn test_send_message_basic() {
        let msg: ClientMessage = parse_msg(
            r##"{"type": "send_message", "operation_generation": "generation-0001", "channel": "#general", "content": "Hello world"}"##,
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
                ..
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
    fn protocol_v2_mutation_requires_operation_generation() {
        assert!(parse_msg(
            r##"{"type":"send_message","request_id":"request-1","channel":"#general","content":"hello"}"##
        )
        .is_err());
    }

    #[test]
    fn test_send_message_with_reply_and_attachments() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "send_message",
            "operation_generation": "generation-0001",
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
                channel_type,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(name, "new-channel");
                assert!(category_id.is_none());
                assert!(is_private.is_none());
                assert!(channel_type.is_none());
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
            "operation_generation": "generation-0001",
            "message_id": "msg-1",
            "content": "edited content"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::EditMessage {
                message_id,
                content,
                ..
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
            "operation_generation": "generation-0001",
            "message_id": "msg-1"
        }"##,
        )
        .unwrap();
        assert!(
            matches!(msg, ClientMessage::DeleteMessage { message_id, .. } if message_id == "msg-1")
        );
    }

    #[test]
    fn test_add_reaction() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "add_reaction",
            "operation_generation": "generation-0001",
            "message_id": "msg-1",
            "emoji": "\ud83d\udc4d"
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::AddReaction {
                message_id, emoji, ..
            } => {
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
            "operation_generation": "generation-0001",
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
            "operation_generation": "generation-0001",
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
                ..
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
            "request_id": "request-1",
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
    fn test_invoke_message_component() {
        let msg: ClientMessage = parse_msg(
            r##"{
            "type": "invoke_message_component",
            "request_id": "request-2",
            "message_id": "message-1",
            "custom_id": "priority",
            "values": ["high"]
        }"##,
        )
        .unwrap();
        match msg {
            ClientMessage::InvokeMessageComponent {
                request_id,
                message_id,
                custom_id,
                values,
            } => {
                assert_eq!(request_id, "request-2");
                assert_eq!(message_id, "message-1");
                assert_eq!(custom_id, "priority");
                assert_eq!(values, ["high"]);
            }
            _ => panic!("Expected InvokeMessageComponent"),
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
                client_type,
            } => {
                assert_eq!(name, "My App");
                assert_eq!(description, Some("A cool app".into()));
                assert_eq!(redirect_uris, vec!["https://example.com/callback"]);
                assert_eq!(client_type, "confidential");
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
