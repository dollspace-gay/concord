use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::auth::token::validate_session_token;
use crate::db::queries::users;
use crate::engine::chat_engine::{ChatEngine, DEFAULT_SERVER_ID};
use crate::engine::events::ChatEvent;
use crate::engine::user_session::Protocol;

use super::app_state::AppState;

/// Query parameters for WebSocket upgrade.
/// If authenticated via cookie, nickname is looked up from the user's profile.
/// Falls back to ?nickname= for unauthenticated dev/test usage.
#[derive(Deserialize, Default)]
pub struct WsParams {
    pub nickname: Option<String>,
}

/// Client-to-server WebSocket message types.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    SendMessage {
        #[serde(default = "default_server_id")]
        server_id: String,
        channel: String,
        content: String,
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
    },
    DeleteChannel {
        server_id: String,
        channel: String,
    },
    DeleteServer {
        server_id: String,
    },
    UpdateMemberRole {
        server_id: String,
        user_id: String,
        role: String,
    },
}

fn default_server_id() -> String {
    DEFAULT_SERVER_ID.to_string()
}

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Try cookie-based auth first
    let (nickname, user_id) = if let Some(cookie) = jar.get("concord_session") {
        if let Ok(claims) =
            validate_session_token(cookie.value(), &state.auth_config.jwt_secret)
        {
            match users::get_user(&state.db, &claims.sub).await {
                Ok(Some((id, username, _email, _avatar))) => (username, Some(id)),
                _ => {
                    return (
                        axum::http::StatusCode::UNAUTHORIZED,
                        "User not found",
                    )
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
    } else if let Some(nick) = params.nickname {
        // Fallback: allow ?nickname= for dev/test (no auth required)
        (nick, None)
    } else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Not authenticated. Provide a session cookie or ?nickname= param.",
        )
            .into_response();
    };

    // Look up avatar_url from DB if authenticated via cookie
    let avatar_url = if jar.get("concord_session").is_some() {
        if let Ok(claims) =
            validate_session_token(jar.get("concord_session").unwrap().value(), &state.auth_config.jwt_secret)
        {
            match users::get_user(&state.db, &claims.sub).await {
                Ok(Some((_id, _username, _email, avatar))) => avatar,
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    let engine = state.engine.clone();
    ws.on_upgrade(move |socket| handle_ws_connection(socket, engine, user_id, nickname, avatar_url))
        .into_response()
}

async fn handle_ws_connection(
    socket: WebSocket,
    engine: Arc<ChatEngine>,
    user_id: Option<String>,
    nickname: String,
    avatar_url: Option<String>,
) {
    let (session_id, mut event_rx) = match engine.connect(user_id, nickname.clone(), Protocol::WebSocket, avatar_url) {
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
    while let Some(msg_result) = ws_receiver.next().await {
        let msg = match msg_result {
            Ok(msg) => msg,
            Err(e) => {
                warn!(error = %e, "WebSocket read error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
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
        ClientMessage::SendMessage { server_id, channel, content } => {
            engine.send_message(session_id, &server_id, &channel, &content)
        }
        ClientMessage::JoinChannel { server_id, channel } => {
            engine.join_channel(session_id, &server_id, &channel)
        }
        ClientMessage::PartChannel { server_id, channel, reason } => {
            engine.part_channel(session_id, &server_id, &channel, reason)
        }
        ClientMessage::SetTopic { server_id, channel, topic } => {
            engine.set_topic(session_id, &server_id, &channel, topic)
        }
        ClientMessage::FetchHistory {
            server_id,
            channel,
            before,
            limit,
        } => {
            let limit = limit.unwrap_or(50).min(200);
            match engine
                .fetch_history(&server_id, &channel, before.as_deref(), limit)
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
        ClientMessage::ListChannels { server_id } => {
            let channels = engine.list_channels(&server_id);
            if let Some(session) = engine.get_session(session_id) {
                let _ = session.send(ChatEvent::ChannelList { server_id, channels });
            }
            Ok(())
        }
        ClientMessage::GetMembers { server_id, channel } => {
            match engine.get_members(&server_id, &channel) {
                Ok(member_infos) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::Names { server_id, channel, members: member_infos });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::ListServers => {
            if let Some(session) = engine.get_session(session_id) {
                let servers = if let Some(ref uid) = session.user_id {
                    engine.list_servers_for_user(uid)
                } else {
                    engine.list_all_servers()
                };
                let _ = session.send(ChatEvent::ServerList { servers });
            }
            Ok(())
        }
        ClientMessage::CreateServer { name, icon_url } => {
            let session = engine.get_session(session_id);
            let user_id = session.as_ref().and_then(|s| s.user_id.clone());
            let Some(uid) = user_id else {
                return send_error(engine, session_id, "AUTH_REQUIRED", "Must be authenticated to create a server");
            };
            match engine.create_server(name, uid, icon_url).await {
                Ok(_server_id) => {
                    if let Some(session) = engine.get_session(session_id) {
                        if let Some(ref uid) = session.user_id {
                            let servers = engine.list_servers_for_user(uid);
                            let _ = session.send(ChatEvent::ServerList { servers });
                        }
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
                return send_error(engine, session_id, "AUTH_REQUIRED", "Must be authenticated to join a server");
            };
            match engine.join_server(&uid, &server_id).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id) {
                        if let Some(ref uid) = session.user_id {
                            let servers = engine.list_servers_for_user(uid);
                            let _ = session.send(ChatEvent::ServerList { servers });
                        }
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
                return send_error(engine, session_id, "AUTH_REQUIRED", "Must be authenticated to leave a server");
            };
            match engine.leave_server(&uid, &server_id).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id) {
                        if let Some(ref uid) = session.user_id {
                            let servers = engine.list_servers_for_user(uid);
                            let _ = session.send(ChatEvent::ServerList { servers });
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::CreateChannel { server_id, name } => {
            match engine.create_channel_in_server(&server_id, &name).await {
                Ok(_) => {
                    let channels = engine.list_channels(&server_id);
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::ChannelList { server_id, channels });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteChannel { server_id, channel } => {
            match engine.delete_channel_in_server(&server_id, &channel).await {
                Ok(()) => {
                    let channels = engine.list_channels(&server_id);
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::ChannelList { server_id, channels });
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::DeleteServer { server_id } => {
            let session = engine.get_session(session_id);
            let user_id = session.as_ref().and_then(|s| s.user_id.clone());
            let Some(uid) = user_id else {
                return send_error(engine, session_id, "AUTH_REQUIRED", "Must be authenticated to delete a server");
            };
            if !engine.is_server_owner(&server_id, &uid) {
                return send_error(engine, session_id, "FORBIDDEN", "Only the server owner can delete it");
            }
            match engine.delete_server(&server_id).await {
                Ok(()) => {
                    if let Some(session) = engine.get_session(session_id) {
                        if let Some(ref uid) = session.user_id {
                            let servers = engine.list_servers_for_user(uid);
                            let _ = session.send(ChatEvent::ServerList { servers });
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ClientMessage::UpdateMemberRole { server_id, user_id, role } => {
            if let Some(pool) = engine.db() {
                crate::db::queries::servers::update_member_role(pool, &server_id, &user_id, &role)
                    .await
                    .map_err(|e| format!("Failed to update role: {e}"))
            } else {
                Err("No database configured".into())
            }
        }
    };

    if let Err(e) = result {
        send_error(engine, session_id, "COMMAND_FAILED", &e);
    }
}

fn send_error(engine: &ChatEngine, session_id: crate::engine::events::SessionId, code: &str, message: &str) {
    if let Some(session) = engine.get_session(session_id) {
        let _ = session.send(ChatEvent::Error {
            code: code.into(),
            message: message.into(),
        });
    }
}
