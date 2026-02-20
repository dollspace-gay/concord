use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use super::app_state::AppState;
use super::rate_limit::{ApiRateLimiters, api_rate_limit, auth_rate_limit, ws_rate_limit};
use super::{atproto, oauth, rest_api, ws_handler};

/// Build the axum router with all HTTP and WebSocket routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Restrict CORS to the configured public_url origin (or allow any for localhost dev)
    let public_url = &state.auth_config.public_url;
    let cors = if public_url.contains("localhost") || public_url.contains("127.0.0.1") {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        let origin = public_url
            .parse::<HeaderValue>()
            .unwrap_or_else(|_| HeaderValue::from_static("https://localhost"));
        CorsLayer::new()
            .allow_origin(origin)
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let rate_limiters = Arc::new(ApiRateLimiters::default());

    // Auth routes — tight rate limit to prevent brute force
    let auth_routes = Router::new()
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
        .route(
            "/api/auth/atproto/login",
            axum::routing::get(atproto::atproto_login),
        )
        .route(
            "/api/auth/atproto/callback",
            axum::routing::get(atproto::atproto_callback),
        )
        .route("/api/auth/logout", axum::routing::post(oauth::logout))
        .layer(axum::middleware::from_fn(auth_rate_limit));

    // WebSocket — connection rate limit
    let ws_routes = Router::new()
        .route("/ws", axum::routing::get(ws_handler::ws_upgrade))
        .layer(axum::middleware::from_fn(ws_rate_limit));

    // All other API routes — general rate limit
    let api_routes = Router::new()
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
            axum::routing::post(rest_api::upload_file)
                .layer(DefaultBodyLimit::max(state.max_file_size as usize)),
        )
        .route(
            "/api/uploads/{id}",
            axum::routing::get(rest_api::get_upload),
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
        // Webhook incoming execution (public, token in URL)
        .route(
            "/api/webhooks/{id}/{token}",
            axum::routing::post(rest_api::execute_webhook),
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
        // AT Protocol record sync settings
        .route(
            "/api/settings/atproto-sync",
            axum::routing::get(rest_api::get_atproto_sync_setting)
                .patch(rest_api::update_atproto_sync_setting),
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
        // Server limits (public)
        .route(
            "/api/config/limits",
            axum::routing::get(rest_api::get_server_limits),
        )
        .layer(axum::middleware::from_fn(api_rate_limit));

    Router::new()
        .merge(ws_routes)
        .merge(auth_routes)
        .merge(api_routes)
        // Static files with SPA fallback — unmatched routes serve index.html
        .fallback_service(ServeDir::new("static").fallback(ServeFile::new("static/index.html")))
        .layer(cors)
        // Inject rate limiters into all request extensions
        .layer(axum::Extension(rate_limiters))
        .with_state(state)
}
