use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::engine::chat_engine::ChatEngine;
use crate::engine::events::ChatEvent;
use crate::engine::user_session::Protocol;

/// Query parameters for WebSocket upgrade — nickname is required for now.
/// Once OAuth is implemented, this will be replaced with token-based auth.
#[derive(Deserialize)]
pub struct WsParams {
    pub nickname: String,
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
    ListChannels,
    GetMembers {
        channel: String,
    },
}

pub async fn ws_upgrade(
    State(engine): State<Arc<ChatEngine>>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let nickname = params.nickname;
    ws.on_upgrade(move |socket| handle_ws_connection(socket, engine, nickname))
}

async fn handle_ws_connection(socket: WebSocket, engine: Arc<ChatEngine>, nickname: String) {
    // Register this session with the engine
    let (session_id, mut event_rx) = match engine.connect(nickname.clone(), Protocol::WebSocket) {
        Ok(pair) => pair,
        Err(e) => {
            warn!(%nickname, error = %e, "WebSocket connection rejected");
            // Can't send error before upgrade completes cleanly, just drop
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();

    use futures_util::{SinkExt, StreamExt};

    // Spawn write loop: engine events -> WebSocket frames
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

    // Read loop: WebSocket frames -> engine commands
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
                handle_client_message(&engine_ref, session_id, &text);
            }
            Message::Close(_) => break,
            _ => {} // Ignore binary, ping, pong (axum handles ping/pong)
        }
    }

    // Clean up
    engine.disconnect(session_id);
    write_handle.abort();
    info!(%session_id, %nickname, "WebSocket connection closed");
}

fn handle_client_message(
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
                    let _ = session.send(ChatEvent::Names {
                        channel,
                        members,
                    });
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
