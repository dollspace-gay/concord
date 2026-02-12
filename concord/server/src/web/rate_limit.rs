use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::engine::rate_limiter::RateLimiter;

/// Per-IP rate limiters for different endpoint tiers.
pub struct ApiRateLimiters {
    /// Auth endpoints (login, callback): tight limit to prevent brute force.
    /// Burst of 10, refill 1 per 6 seconds (~10/minute).
    pub auth: RateLimiter,
    /// General API endpoints: moderate limit.
    /// Burst of 60, refill 1 per second (~60/minute sustained).
    pub api: RateLimiter,
    /// WebSocket connections: prevent connection storms.
    /// Burst of 5, refill 1 per 12 seconds (~5/minute).
    pub ws: RateLimiter,
}

impl Default for ApiRateLimiters {
    fn default() -> Self {
        Self {
            auth: RateLimiter::new(10, 6.0),
            api: RateLimiter::new(60, 1.0),
            ws: RateLimiter::new(5, 12.0),
        }
    }
}

/// Extract client IP from request headers or connection info.
fn client_ip(req: &Request<Body>) -> String {
    // Check X-Forwarded-For first (for reverse proxies / ngrok)
    if let Some(forwarded) = req.headers().get("x-forwarded-for")
        && let Ok(val) = forwarded.to_str()
        && let Some(first) = val.split(',').next()
    {
        return first.trim().to_string();
    }

    // Check X-Real-IP
    if let Some(real_ip) = req.headers().get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
    {
        return val.trim().to_string();
    }

    // Fallback: use a generic key (all connections share the limit)
    "unknown".to_string()
}

/// Middleware for auth endpoint rate limiting.
pub async fn auth_rate_limit(req: Request<Body>, next: Next) -> Response {
    let limiters = req.extensions().get::<Arc<ApiRateLimiters>>();
    if let Some(limiters) = limiters {
        let ip = client_ip(&req);
        if !limiters.auth.check(&ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Please try again later.",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Middleware for general API rate limiting.
pub async fn api_rate_limit(req: Request<Body>, next: Next) -> Response {
    let limiters = req.extensions().get::<Arc<ApiRateLimiters>>();
    if let Some(limiters) = limiters {
        let ip = client_ip(&req);
        if !limiters.api.check(&ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Please try again later.",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Middleware for WebSocket connection rate limiting.
pub async fn ws_rate_limit(req: Request<Body>, next: Next) -> Response {
    let limiters = req.extensions().get::<Arc<ApiRateLimiters>>();
    if let Some(limiters) = limiters {
        let ip = client_ip(&req);
        if !limiters.ws.check(&ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many connections. Please try again later.",
            )
                .into_response();
        }
    }
    next.run(req).await
}
