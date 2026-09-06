use std::sync::Arc;
use concord_server::{db::{pool::{create_pool, run_migrations}, queries}, engine::{chat_engine::ChatEngine, user_session::Protocol, events::ChatEvent, permissions::Permissions}, auth::{config::AuthConfig, token::{create_session_token, validate_session_token, JwtBlocklist}}, web::{app_state::AppState, atproto::AtprotoOAuth, router::build_router}};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = create_pool("sqlite::memory:").await?;
    run_migrations(&pool).await?;
    for name in ["owner", "member", "outsider"] {
        queries::users::create_with_oauth(&pool, &queries::users::CreateOAuthUser { user_id:name, username:name, email:None, avatar_url:None, oauth_id:name, provider:"test", provider_id:name }).await?;
    }
    let engine = Arc::new(ChatEngine::new(Some(pool.clone()), 4000, 100));
    let server = engine.create_server("Review".into(), "owner".into(), None).await?;
    engine.join_server("member", &server).await?;
    let channel = engine.create_channel_in_server(&server, "#secret", None, true).await?;
    queries::channels::set_channel_override(&pool, "deny-member", &channel, "user", "member", 0, Permissions::VIEW_CHANNELS.bits() as i64).await?;
    assert!(!engine.get_effective_permissions(&server, Some(&channel), "member").await.contains(Permissions::VIEW_CHANNELS));
    let message_id = uuid::Uuid::new_v4().to_string();
    queries::messages::insert_message(&pool, &queries::messages::InsertMessageParams { id:&message_id, server_id:&server, channel_id:&channel, sender_id:"owner", sender_nick:"owner", content:"reviewsecret", reply_to_id:None }).await?;
    let secret = "isolated-review-secret-not-a-real-credential";
    let atproto = AtprotoOAuth::load_or_create(&pool).await;
    let state = Arc::new(AppState { engine:engine.clone(), db:pool.clone(), auth_config:AuthConfig { jwt_secret:secret.into(), session_expiry_hours:1, public_url:"http://localhost".into() }, atproto, max_file_size:1024, max_message_length:4000, jwt_blocklist:JwtBlocklist::new() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = build_router(state.clone());
    let server_task = tokio::spawn(async move { axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap(); });
    let client = reqwest::Client::new();
    let member_token = create_session_token("member", secret, 1)?;
    let response = client.get(format!("http://{addr}/api/search?server_id={server}&q=reviewsecret"))
        .header("cookie", format!("concord_session={member_token}")).send().await?;
    let status = response.status();
    let body = response.text().await?;
    println!("REST private-channel search: status={status}, leaked_secret={}", body.contains("reviewsecret"));
    assert!(body.contains("reviewsecret"), "response: {body}");
    let outsider_token = create_session_token("outsider", secret, 1)?;
    let claims = validate_session_token(&outsider_token, secret)?;
    state.jwt_blocklist.revoke(&claims.jti, claims.exp);
    let rest = client.get(format!("http://{addr}/api/me")).header("cookie", format!("concord_session={outsider_token}")).send().await?;
    println!("Revoked token REST /api/me: {}", rest.status());
    assert_eq!(rest.status(), reqwest::StatusCode::UNAUTHORIZED);
    let mut request = format!("ws://{addr}/ws").into_client_request()?;
    request.headers_mut().insert("cookie", format!("concord_session={outsider_token}").parse()?);
    let (mut ws, response) = tokio_tungstenite::connect_async(request).await?;
    println!("Revoked token WebSocket upgrade: {}", response.status());
    ws.send(Message::Text(serde_json::json!({"type":"search_messages", "server_id":server, "query":"reviewsecret"}).to_string().into())).await?;
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await?.ok_or("WebSocket closed")??;
    println!("Nonmember WebSocket search: leaked_secret={}", response.to_string().contains("reviewsecret"));
    assert!(response.to_string().contains("reviewsecret"));
    ws.close(None).await?;
    let (old, _old_rx) = engine.connect(Some("owner".into()), "owner".into(), Protocol::WebSocket, None)?;
    let (_new, _new_rx) = engine.connect(Some("owner".into()), "owner".into(), Protocol::Irc, None)?;
    println!("Web + IRC same identity: original_session_survives={}", engine.get_session(old).is_some());
    assert!(engine.get_session(old).is_none());
    let (sender, mut rx) = engine.connect(Some("owner".into()), "owner".into(), Protocol::WebSocket, None)?;
    engine.join_channel(sender, &server, "#general")?;
    while rx.try_recv().is_ok() {}
    sqlx::query("CREATE TRIGGER review_reject BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'injected storage failure'); END").execute(&pool).await?;
    engine.send_message(sender, &server, "#general", "rejectedwrite", None, None, Some("review-nonce"))?;
    let mut acknowledged = false;
    while let Ok(event) = rx.try_recv() { if matches!(event, ChatEvent::MessageAck { .. }) { acknowledged = true; } }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let count:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE content = 'rejectedwrite'").fetch_one(&pool).await?;
    println!("Injected INSERT failure: acknowledged={acknowledged}, persisted_rows={count}");
    assert!(acknowledged && count == 0);
    server_task.abort();
    Ok(())
}
