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
use crate::engine::chat_engine::ChatEngine;
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
        channel: String,
        content: String,
    },
    JoinChannel {
        channel: String,
    },
    PartChannel {
        channel: String,
        reason: Option<String>,
    },
    SetTopic {
        channel: String,
        topic: String,
    },
    FetchHistory {
        channel: String,
        before: Option<String>,
        limit: Option<i64>,
    },
    ListChannels,
    GetMembers {
        channel: String,
    },
}

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Try cookie-based auth first
    let nickname = if let Some(cookie) = jar.get("concord_session") {
        if let Ok(claims) =
            validate_session_token(cookie.value(), &state.auth_config.jwt_secret)
        {
            match users::get_user(&state.db, &claims.sub).await {
                Ok(Some((_id, username, _email, _avatar))) => username,
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
        nick
    } else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Not authenticated. Provide a session cookie or ?nickname= param.",
        )
            .into_response();
    };

    let engine = state.engine.clone();
    ws.on_upgrade(move |socket| handle_ws_connection(socket, engine, nickname))
        .into_response()
}

async fn handle_ws_connection(socket: WebSocket, engine: Arc<ChatEngine>, nickname: String) {
    let (session_id, mut event_rx) = match engine.connect(nickname.clone(), Protocol::WebSocket) {
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
        ClientMessage::SendMessage { channel, content } => {
            engine.send_message(session_id, &channel, &content)
        }
        ClientMessage::JoinChannel { channel } => engine.join_channel(session_id, &channel),
        ClientMessage::PartChannel { channel, reason } => {
            engine.part_channel(session_id, &channel, reason)
        }
        ClientMessage::SetTopic { channel, topic } => {
            engine.set_topic(session_id, &channel, topic)
        }
        ClientMessage::FetchHistory {
            channel,
            before,
            limit,
        } => {
            let limit = limit.unwrap_or(50).min(200);
            match engine
                .fetch_history(&channel, before.as_deref(), limit)
                .await
            {
                Ok((messages, has_more)) => {
                    if let Some(session) = engine.get_session(session_id) {
                        let _ = session.send(ChatEvent::History {
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
        ClientMessage::ListChannels => {
            let channels = engine.list_channels();
            if let Some(session) = engine.get_session(session_id) {
                let _ = session.send(ChatEvent::ChannelList { channels });
            }
            Ok(())
        }
        ClientMessage::GetMembers { channel } => match engine.get_members(&channel) {
            Ok(members) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::Names { channel, members });
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
    };

    if let Err(e) = result {
        warn!(%session_id, error = %e, "command failed");
        if let Some(session) = engine.get_session(session_id) {
            let _ = session.send(ChatEvent::Error {
                code: "COMMAND_FAILED".into(),
                message: e,
            });
        }
    }
}
