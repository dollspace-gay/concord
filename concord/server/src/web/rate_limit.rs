use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::extract::Request;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::app_state::AppState;
use crate::engine::rate_limiter::RateLimiter;

/// G22 requires 200 reconnecting sessions behind one address to recover within
/// 30 seconds. These finite budgets leave room for transport retries while
/// bounding each source address, credential, and the process as a whole.
const WS_RECONNECT_BURST: u32 = 256;
const WS_GLOBAL_BURST: u32 = 2048;
const WS_REFILL_SECONDS: f64 = 0.125;
const AUTHENTICATED_API_BURST: u32 = 256;
const API_GLOBAL_BURST: u32 = 4096;
const API_REFILL_SECONDS: f64 = 0.125;

/// Per-IP rate limiters for different endpoint tiers.
pub struct ApiRateLimiters {
    /// Auth endpoints (login, callback): tight limit to prevent brute force.
    /// Burst of 32 for complete PKCE/refresh/retry flows, refill 1 per 6
    /// seconds (~10/minute sustained).
    pub auth: RateLimiter,
    /// General API endpoints: moderate limit.
    /// Burst of 60, refill 1 per second (~60/minute sustained).
    pub api_pre_auth_ip: RateLimiter,
    /// General API requests made with a validated credential.
    pub api_authenticated: RateLimiter,
    /// Process-wide API ceiling, independent of source or credential keys.
    pub api_global: RateLimiter,
    /// WebSocket handshakes before authentication, keyed by source address.
    pub ws_pre_auth_ip: RateLimiter,
    /// Process-wide WebSocket handshake ceiling, independent of source keys.
    pub ws_pre_auth_global: RateLimiter,
    /// Validated WebSocket handshakes, keyed by stable credential identity.
    pub ws_authenticated: RateLimiter,
    /// Webhook execution: tighter limit to prevent argon2 DoS.
    /// Burst of 10, refill 1 per 3 seconds (~20/minute per IP).
    pub webhook: RateLimiter,
}

impl Default for ApiRateLimiters {
    fn default() -> Self {
        Self {
            auth: RateLimiter::new(32, 6.0),
            api_pre_auth_ip: RateLimiter::new(60, 1.0),
            api_authenticated: RateLimiter::new(AUTHENTICATED_API_BURST, API_REFILL_SECONDS),
            api_global: RateLimiter::new(API_GLOBAL_BURST, API_REFILL_SECONDS),
            ws_pre_auth_ip: RateLimiter::new(WS_RECONNECT_BURST, WS_REFILL_SECONDS),
            ws_pre_auth_global: RateLimiter::new(WS_GLOBAL_BURST, WS_REFILL_SECONDS),
            ws_authenticated: RateLimiter::new(WS_RECONNECT_BURST, WS_REFILL_SECONDS),
            webhook: RateLimiter::new(10, 3.0),
        }
    }
}

impl ApiRateLimiters {
    pub fn admit_authenticated_ws(&self, credential_id: &str) -> bool {
        self.ws_authenticated.check(credential_id)
    }

    pub fn admit_authenticated_api(&self, credential_id: &str) -> bool {
        self.api_authenticated.check(credential_id)
    }

    pub fn admit_global_api(&self) -> bool {
        self.api_global.check("all")
    }

    pub fn admit_credential_verification(&self, ip: &str) -> bool {
        self.api_pre_auth_ip.check(ip)
    }
}

/// Extract the direct peer IP. Forwarding headers are ignored until an operator
/// explicitly configures a trusted proxy boundary.
fn client_ip(req: &Request<Body>) -> String {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    // Use actual peer IP, or fall back to "unknown" if ConnectInfo is unavailable
    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string())
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
pub async fn api_rate_limit(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let limiters = req.extensions().get::<Arc<ApiRateLimiters>>();
    if let Some(limiters) = limiters {
        if !limiters.admit_global_api() {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Please try again later.",
            )
                .into_response();
        }
        // Bearer credentials may require bounded database/password-hash work.
        // Cookie JWT verification is cheap and valid cookie requests are
        // isolated by credential below, so they must not share an anonymous
        // source-address bucket (many users can legitimately share one NAT).
        let has_bearer = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .is_some();
        let has_cookie = req.headers().get(axum::http::header::COOKIE).is_some();
        if (has_bearer || !has_cookie) && !limiters.admit_credential_verification(&client_ip(&req))
        {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Please try again later.",
            )
                .into_response();
        }
        let principal = match super::auth_middleware::request_principal(&state, req.headers()).await
        {
            Ok(principal) => principal,
            Err(response) => return response,
        };
        if let Some(principal) = principal
            && !limiters.admit_authenticated_api(principal.credential_key())
        {
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
        if !limiters.ws_pre_auth_global.check("all") || !limiters.ws_pre_auth_ip.check(&ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many connections. Please try again later.",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Middleware for webhook execution rate limiting.
/// Webhooks use argon2 token verification which is CPU-expensive,
/// so we apply a tighter rate limit to prevent DoS attacks.
pub async fn webhook_rate_limit(req: Request<Body>, next: Next) -> Response {
    let limiters = req.extensions().get::<Arc<ApiRateLimiters>>();
    if let Some(limiters) = limiters {
        let ip = client_ip(&req);
        if !limiters.webhook.check(&ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Please try again later.",
            )
                .into_response();
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn websocket_admission_supports_qualification_burst_behind_one_address() {
        let limiters = ApiRateLimiters::default();
        for index in 0..200 {
            assert!(limiters.ws_pre_auth_global.check("all"));
            assert!(limiters.ws_pre_auth_ip.check("203.0.113.1"));
            assert!(limiters.admit_authenticated_ws(&format!("credential-{index}")));
        }
    }

    #[test]
    fn websocket_admission_remains_bounded_per_credential() {
        let limiters = ApiRateLimiters::default();
        for _ in 0..WS_RECONNECT_BURST {
            assert!(limiters.admit_authenticated_ws("one-credential"));
        }
        assert!(!limiters.admit_authenticated_ws("one-credential"));
    }

    #[test]
    fn authenticated_api_admission_is_principal_scoped_and_globally_bounded() {
        let limiters = ApiRateLimiters::default();
        for index in 0..200 {
            assert!(limiters.admit_authenticated_api(&format!("credential-{index}")));
        }
        for _ in 0..AUTHENTICATED_API_BURST {
            assert!(limiters.api_authenticated.check("one-credential"));
        }
        assert!(!limiters.api_authenticated.check("one-credential"));
    }

    #[test]
    fn shared_source_does_not_spend_anonymous_budget_for_validated_principals() {
        let limiters = ApiRateLimiters::default();
        for index in 0..200 {
            assert!(limiters.admit_global_api());
            assert!(limiters.admit_authenticated_api(&format!("credential-{index}")));
        }
        for _ in 0..60 {
            assert!(limiters.admit_credential_verification("203.0.113.1"));
        }
        assert!(!limiters.admit_credential_verification("203.0.113.1"));
    }
}
