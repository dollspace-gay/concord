use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use super::app_state::AppState;
use super::rate_limit::{
    ApiRateLimiters, api_rate_limit, auth_rate_limit, webhook_rate_limit, ws_rate_limit,
};
use super::{atproto, oauth, observability, rest_api, ws_handler};

/// Multipart framing is bounded separately from the configured file-byte
/// limit. MediaUpload still enforces the exact configured limit on file bytes.
pub(crate) const MULTIPART_BODY_OVERHEAD_BYTES: usize = 64 * 1024;

fn upload_body_limit(max_file_size: u64) -> usize {
    usize::try_from(max_file_size)
        .unwrap_or(usize::MAX)
        .saturating_add(MULTIPART_BODY_OVERHEAD_BYTES)
}

/// Middleware that adds security response headers to every response.
async fn security_headers(req: axum::extract::Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert(
        "referrer-policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    resp
}

/// Build the axum router with all HTTP and WebSocket routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Restrict CORS to the exact configured origin. The development frontend
    // proxies API and WebSocket requests, so it does not require a wildcard.
    let public_url = &state.auth_config.public_url;
    let origin = public_url
        .parse::<HeaderValue>()
        .unwrap_or_else(|_| HeaderValue::from_static("https://localhost"));
    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_methods(Any)
        .allow_headers(Any);

    let rate_limiters = Arc::new(ApiRateLimiters::default());

    // Auth routes — tight rate limit to prevent brute force
    let auth_discovery_routes = Router::new()
        .route(
            "/api/auth/status",
            axum::routing::get(rest_api::auth_status),
        )
        .route(
            "/api/auth/atproto/client-metadata.json",
            axum::routing::get(atproto::client_metadata),
        )
        .route(
            "/api/auth/atproto/v2/client-metadata.json",
            axum::routing::get(atproto::client_metadata),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            api_rate_limit,
        ));
    let auth_routes = Router::new()
        .route(
            "/api/auth/atproto/login",
            axum::routing::get(atproto::atproto_login),
        )
        .route(
            "/api/auth/atproto/callback",
            axum::routing::get(atproto::atproto_callback),
        )
        .layer(axum::middleware::from_fn(auth_rate_limit));
    let oauth_protocol_routes = Router::new()
        .route(
            "/oauth/authorize",
            axum::routing::get(oauth::authorize_get).post(oauth::authorize_post),
        )
        .route("/api/oauth/token", axum::routing::post(oauth::token))
        .layer(axum::middleware::from_fn(auth_rate_limit));
    let logout_route = Router::new()
        .route("/api/auth/logout", axum::routing::post(oauth::logout))
        .layer(axum::middleware::from_fn(auth_rate_limit));

    // WebSocket — connection rate limit
    let ws_routes = Router::new()
        .route("/ws", axum::routing::get(ws_handler::ws_upgrade))
        .layer(axum::middleware::from_fn(ws_rate_limit));

    // All other API routes — general rate limit
    let api_routes = Router::new()
        .route("/api/oauth/userinfo", axum::routing::get(oauth::userinfo))
        .route(
            "/api/oauth/servers",
            axum::routing::get(oauth::delegated_servers),
        )
        .route(
            "/api/oauth/grants/{id}/revoke",
            axum::routing::post(oauth::revoke_grant),
        )
        // Public channel endpoints (default server, backward compat)
        .route("/api/channels", axum::routing::get(rest_api::get_channels))
        .route(
            "/api/channels/{name}/messages",
            axum::routing::get(rest_api::get_channel_history),
        )
        // Server endpoints (authenticated)
        .route(
            "/api/servers",
            axum::routing::get(rest_api::list_servers).post(rest_api::create_server),
        )
        .route(
            "/api/servers/{id}",
            axum::routing::get(rest_api::get_server).delete(rest_api::delete_server),
        )
        .route(
            "/api/servers/{id}/channels",
            axum::routing::get(rest_api::list_server_channels),
        )
        .route(
            "/api/servers/{id}/channels/{name}/messages",
            axum::routing::get(rest_api::get_server_channel_history),
        )
        .route(
            "/api/servers/{id}/members",
            axum::routing::get(rest_api::list_server_members),
        )
        // Admin endpoints (system admin only)
        .route(
            "/api/admin/servers",
            axum::routing::get(rest_api::admin_list_servers),
        )
        .route(
            "/api/admin/servers/{id}",
            axum::routing::delete(rest_api::admin_delete_server),
        )
        .route(
            "/api/admin/users/{id}/admin",
            axum::routing::put(rest_api::admin_set_admin),
        )
        // User profile lookup (public)
        .route(
            "/api/users/{nickname}",
            axum::routing::get(rest_api::get_user_profile),
        )
        // Authenticated user endpoints
        .route("/api/me", axum::routing::get(rest_api::get_me))
        .route(
            "/api/tokens",
            axum::routing::get(rest_api::list_irc_tokens).post(rest_api::create_irc_token),
        )
        .route(
            "/api/tokens/{id}",
            axum::routing::delete(rest_api::delete_irc_token),
        )
        // File upload/download
        .route(
            "/api/uploads",
            axum::routing::post(rest_api::upload_file).layer(DefaultBodyLimit::max(
                upload_body_limit(state.max_file_size),
            )),
        )
        .route(
            "/api/uploads/{id}",
            axum::routing::get(rest_api::get_upload).delete(rest_api::delete_upload),
        )
        // Custom emoji
        .route(
            "/api/servers/{id}/emoji",
            axum::routing::get(rest_api::list_server_emoji).post(rest_api::create_server_emoji),
        )
        .route(
            "/api/servers/{id}/emoji/{emoji_id}",
            axum::routing::delete(rest_api::delete_server_emoji),
        )
        .route(
            "/api/servers/{id}/media",
            axum::routing::patch(rest_api::update_server_media),
        )
        .route(
            "/api/servers/{id}/member-media",
            axum::routing::patch(rest_api::update_server_member_media),
        )
        // User profiles
        .route(
            "/api/users/{id}/profile",
            axum::routing::get(rest_api::get_user_full_profile),
        )
        .route(
            "/api/profile",
            axum::routing::patch(rest_api::update_profile),
        )
        // Search
        .route("/api/search", axum::routing::get(rest_api::search_messages))
        // Invite preview (public)
        .route(
            "/api/invite/{code}",
            axum::routing::get(rest_api::get_invite_preview),
        )
        // Server discovery (public)
        .route(
            "/api/discover",
            axum::routing::get(rest_api::discover_servers),
        )
        // Bluesky / AT Protocol integration
        .route(
            "/api/bluesky/sync-profile",
            axum::routing::post(rest_api::sync_bluesky_profile),
        )
        .route(
            "/api/users/{id}/bluesky",
            axum::routing::get(rest_api::get_bluesky_identity),
        )
        .route(
            "/api/messages/{id}/share-bluesky",
            axum::routing::post(rest_api::share_to_bluesky),
        )
        .route(
            "/api/atproto/publications",
            axum::routing::get(rest_api::list_atproto_publications),
        )
        .route(
            "/api/atproto/publications/{id}/retry",
            axum::routing::post(rest_api::retry_atproto_publication),
        )
        // AT Protocol record sync settings
        .route(
            "/api/settings/atproto-sync",
            axum::routing::get(rest_api::get_atproto_sync_setting)
                .patch(rest_api::update_atproto_sync_setting),
        )
        .route(
            "/api/channels/{id}/atproto-publication",
            axum::routing::get(rest_api::get_atproto_channel_policy)
                .patch(rest_api::configure_atproto_channel),
        )
        // Stickers
        .route(
            "/api/servers/{id}/stickers",
            axum::routing::get(rest_api::list_server_stickers)
                .post(rest_api::create_server_sticker),
        )
        .route(
            "/api/servers/{id}/stickers/{sticker_id}",
            axum::routing::delete(rest_api::delete_server_sticker),
        )
        // Cross-server emoji (all emoji for a user across servers)
        .route(
            "/api/users/me/emoji",
            axum::routing::get(rest_api::list_user_emoji),
        )
        // Emoji sharing settings
        .route(
            "/api/servers/{id}/emoji-settings",
            axum::routing::patch(rest_api::update_emoji_settings),
        )
        .route(
            "/api/server-folders",
            axum::routing::get(rest_api::get_server_folders).put(rest_api::replace_server_folders),
        )
        // Server limits (public)
        .route(
            "/api/config/limits",
            axum::routing::get(rest_api::get_server_limits),
        )
        .route(
            "/api/webhooks/{id}/deliveries",
            axum::routing::get(rest_api::list_webhook_deliveries),
        )
        .route(
            "/api/webhooks/{id}/test",
            axum::routing::post(rest_api::test_outgoing_webhook),
        )
        .route(
            "/api/webhook-deliveries/{id}/retry",
            axum::routing::post(rest_api::retry_webhook_delivery),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            api_rate_limit,
        ));

    // Webhook execution — dedicated tighter rate limit (argon2 token verification is CPU-expensive)
    let webhook_routes = Router::new()
        .route(
            "/api/webhooks/{id}/{token}",
            axum::routing::post(rest_api::execute_webhook),
        )
        .layer(axum::middleware::from_fn(webhook_rate_limit));

    Router::new()
        .route(
            "/health/live",
            axum::routing::get(observability::health_live),
        )
        .route(
            "/health/ready",
            axum::routing::get(observability::health_ready),
        )
        .route("/metrics", axum::routing::get(observability::metrics))
        .merge(ws_routes)
        .merge(auth_discovery_routes)
        .merge(auth_routes)
        .merge(oauth_protocol_routes)
        .merge(logout_route)
        .merge(webhook_routes)
        .merge(api_routes)
        // Static files with SPA fallback — unmatched routes serve index.html
        .fallback_service(ServeDir::new("static").fallback(ServeFile::new("static/index.html")))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(cors)
        // Inject rate limiters into all request extensions
        .layer(axum::Extension(rate_limiters))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, Bytes};
    use axum::extract::{DefaultBodyLimit, Multipart};
    use axum::http::{Request, StatusCode, header};
    use axum::routing::post;
    use tower::ServiceExt as _;

    use super::upload_body_limit;

    async fn consume_one_file(mut multipart: Multipart) -> Result<StatusCode, StatusCode> {
        let field = multipart
            .next_field()
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .ok_or(StatusCode::BAD_REQUEST)?;
        let bytes: Bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        (bytes.len() == 1_024)
            .then_some(StatusCode::NO_CONTENT)
            .ok_or(StatusCode::BAD_REQUEST)
    }

    #[tokio::test]
    async fn upload_route_accepts_exact_file_limit_with_bounded_multipart_envelope() {
        let boundary = "concord-exact-file-limit";
        let prefix = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"exact.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        );
        let suffix = format!("\r\n--{boundary}--\r\n");
        let mut body = prefix.into_bytes();
        body.extend(std::iter::repeat_n(0x5a, 1_024));
        body.extend(suffix.as_bytes());
        assert!(body.len() > 1_024);
        assert!(body.len() <= upload_body_limit(1_024));

        let router = Router::new()
            .route("/upload", post(consume_one_file))
            .layer(DefaultBodyLimit::max(upload_body_limit(1_024)));
        let response = router
            .oneshot(
                Request::post("/upload")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
