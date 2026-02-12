use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Redirect, Response};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use uuid::Uuid;

use atproto_identity::key::{KeyData, KeyType, generate_key, to_public};
use atproto_oauth::jwk;
use atproto_oauth::pkce;
use atproto_oauth::resources::{AuthorizationServer, pds_resources};
use atproto_oauth::workflow::{
    OAuthClient, OAuthRequest, OAuthRequestState, oauth_complete, oauth_init,
};

use super::app_state::AppState;
use crate::auth::config::AuthConfig;
use crate::auth::token::create_session_token;
use crate::db::queries::users;

/// State for pending AT Protocol OAuth flows.
pub struct AtprotoOAuth {
    /// ES256 private signing key for client assertions.
    pub signing_key: KeyData,
    /// Public JWK for the client metadata document.
    pub public_jwk: jwk::WrappedJsonWebKey,
    /// Pending OAuth requests keyed by state parameter.
    pub pending: Mutex<HashMap<String, PendingAtprotoAuth>>,
}

pub struct PendingAtprotoAuth {
    pub oauth_request: OAuthRequest,
    pub dpop_key: KeyData,
    pub handle: String,
    pub auth_server: AuthorizationServer,
    pub pds_url: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Maximum number of pending OAuth flows at any time.
const MAX_PENDING_OAUTH: usize = 1000;
/// TTL for pending OAuth flows (10 minutes).
const PENDING_OAUTH_TTL_SECS: i64 = 600;

impl AtprotoOAuth {
    /// Load the signing key from the database, or generate and persist a new one.
    pub async fn load_or_create(pool: &sqlx::SqlitePool) -> Self {
        const KEY_NAME: &str = "atproto_signing_key";

        // Try to load existing key from server_config
        let existing: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_config WHERE key = ?")
                .bind(KEY_NAME)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

        let signing_key = if let Some(ref jwk_json) = existing {
            if let Ok(wrapped) = serde_json::from_str::<jwk::WrappedJsonWebKey>(jwk_json) {
                if let Ok(key) = jwk::to_key_data(&wrapped) {
                    info!("loaded persisted AT Protocol signing key");
                    key
                } else {
                    warn!("stored signing key is invalid, generating new one");
                    Self::generate_and_store(pool, KEY_NAME).await
                }
            } else {
                warn!("stored signing key JSON is malformed, generating new one");
                Self::generate_and_store(pool, KEY_NAME).await
            }
        } else {
            info!("no persisted AT Protocol signing key found, generating new one");
            Self::generate_and_store(pool, KEY_NAME).await
        };

        let public_key =
            to_public(&signing_key).expect("failed to derive public key from signing key");
        let public_jwk =
            jwk::generate(&public_key).expect("failed to generate JWK from public key");
        Self {
            signing_key,
            public_jwk,
            pending: Mutex::new(HashMap::new()),
        }
    }

    async fn generate_and_store(pool: &sqlx::SqlitePool, key_name: &str) -> KeyData {
        let signing_key =
            generate_key(KeyType::P256Private).expect("failed to generate atproto signing key");
        let wrapped = jwk::generate(&signing_key).expect("failed to generate JWK for signing key");
        let jwk_json = serde_json::to_string(&wrapped).expect("failed to serialize signing key");

        let _ = sqlx::query(
            "INSERT INTO server_config (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
        )
        .bind(key_name)
        .bind(&jwk_json)
        .execute(pool)
        .await
        .map_err(|e| warn!(error = %e, "failed to persist signing key to database"));

        signing_key
    }
}

/// GET /api/auth/atproto/client-metadata.json — serves OAuth client metadata document.
pub async fn client_metadata(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let public_url = &state.auth_config.public_url;
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);

    let public_jwk_value =
        serde_json::to_value(&state.atproto.public_jwk).expect("failed to serialize public JWK");

    let metadata = serde_json::json!({
        "client_id": client_id,
        "application_type": "web",
        "client_name": "Concord",
        "client_uri": public_url,
        "dpop_bound_access_tokens": true,
        "grant_types": ["authorization_code", "refresh_token"],
        "redirect_uris": [format!("{}/api/auth/atproto/callback", public_url)],
        "response_types": ["code"],
        "scope": "atproto transition:generic",
        "token_endpoint_auth_method": "private_key_jwt",
        "token_endpoint_auth_signing_alg": "ES256",
        "jwks": {
            "keys": [public_jwk_value]
        }
    });

    Json(metadata)
}

#[derive(Deserialize)]
pub struct AtprotoLoginParams {
    pub handle: String,
}

/// GET /api/auth/atproto/login?handle=user.bsky.social — initiate Bluesky OAuth flow.
pub async fn atproto_login(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AtprotoLoginParams>,
) -> Response {
    let handle = params.handle.trim().to_string();
    if handle.is_empty() {
        return (StatusCode::BAD_REQUEST, "Handle is required").into_response();
    }

    let public_url = &state.auth_config.public_url;
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);
    let redirect_uri = format!("{}/api/auth/atproto/callback", public_url);

    let http_client = reqwest::Client::new();

    // Resolve handle -> DID -> DID document -> PDS endpoint
    let pds_url = match resolve_handle_to_pds(&http_client, &handle).await {
        Ok(url) => url,
        Err(e) => {
            warn!(handle = %handle, error = %e, "Failed to resolve handle");
            return (
                StatusCode::BAD_REQUEST,
                format!("Could not resolve handle: {}", e),
            )
                .into_response();
        }
    };

    // Discover authorization server from PDS
    let (_resource, auth_server) = match pds_resources(&http_client, &pds_url).await {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "Failed to fetch PDS resources");
            return (
                StatusCode::BAD_GATEWAY,
                "Failed to discover authorization server",
            )
                .into_response();
        }
    };

    // Generate security parameters
    let dpop_key = generate_key(KeyType::P256Private).expect("failed to generate DPoP key");
    let (pkce_verifier, code_challenge) = pkce::generate();
    let oauth_state = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();

    let oauth_client = OAuthClient {
        redirect_uri: redirect_uri.clone(),
        client_id: client_id.clone(),
        private_signing_key_data: state.atproto.signing_key.clone(),
    };

    let request_state = OAuthRequestState {
        state: oauth_state.clone(),
        nonce: nonce.clone(),
        code_challenge,
        scope: "atproto transition:generic".to_string(),
    };

    // Make Pushed Authorization Request (PAR)
    let par_response = match oauth_init(
        &http_client,
        &oauth_client,
        &dpop_key,
        Some(handle.as_str()),
        &auth_server,
        &request_state,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "PAR request failed");
            return (
                StatusCode::BAD_GATEWAY,
                format!("Authorization request failed: {}", e),
            )
                .into_response();
        }
    };

    // Serialize keys for OAuthRequest storage
    let dpop_jwk = jwk::generate(&dpop_key).unwrap_or_else(|_| {
        panic!("failed to generate DPoP JWK");
    });
    let dpop_private_key = serde_json::to_string(&dpop_jwk).expect("failed to serialize DPoP key");

    let signing_pub =
        to_public(&state.atproto.signing_key).expect("failed to derive signing public key");
    let signing_pub_jwk =
        jwk::generate(&signing_pub).expect("failed to generate signing public JWK");
    let signing_public_key =
        serde_json::to_string(&signing_pub_jwk).expect("failed to serialize signing key");

    let now = Utc::now();

    // Store pending request for callback
    let oauth_request = OAuthRequest {
        oauth_state: oauth_state.clone(),
        issuer: auth_server.issuer.clone(),
        authorization_server: auth_server.issuer.clone(),
        nonce,
        pkce_verifier,
        signing_public_key,
        dpop_private_key,
        created_at: now,
        expires_at: now + chrono::Duration::seconds(par_response.expires_in as i64),
    };

    {
        let mut pending = state.atproto.pending.lock().await;
        // Evict expired entries (older than TTL)
        let cutoff = Utc::now() - chrono::Duration::seconds(PENDING_OAUTH_TTL_SECS);
        pending.retain(|_, v| v.created_at > cutoff);

        // Enforce max pending count
        if pending.len() >= MAX_PENDING_OAUTH {
            warn!(
                "Too many pending OAuth flows ({}), rejecting new request",
                pending.len()
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many pending login requests",
            )
                .into_response();
        }

        pending.insert(
            oauth_state.clone(),
            PendingAtprotoAuth {
                oauth_request,
                dpop_key,
                handle: handle.clone(),
                auth_server: auth_server.clone(),
                pds_url: pds_url.clone(),
                created_at: Utc::now(),
            },
        );
    }

    // Redirect user to authorization server
    let auth_url = format!(
        "{}?client_id={}&request_uri={}",
        auth_server.authorization_endpoint,
        urlencoding::encode(&oauth_client.client_id),
        urlencoding::encode(&par_response.request_uri),
    );

    Redirect::temporary(&auth_url).into_response()
}

#[derive(Deserialize)]
pub struct AtprotoCallbackParams {
    pub code: String,
    pub state: String,
    pub iss: Option<String>,
}

/// GET /api/auth/atproto/callback — exchange code for tokens, create/find user.
pub async fn atproto_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AtprotoCallbackParams>,
) -> Response {
    // Look up pending request
    let pending = {
        let mut pending_map = state.atproto.pending.lock().await;
        pending_map.remove(&params.state)
    };

    let Some(pending) = pending else {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid or expired state parameter",
        )
            .into_response();
    };

    // Verify issuer matches if provided
    if let Some(ref iss) = params.iss
        && *iss != pending.oauth_request.issuer
    {
        return (StatusCode::BAD_REQUEST, "Issuer mismatch").into_response();
    }

    let http_client = reqwest::Client::new();
    let public_url = &state.auth_config.public_url;

    let oauth_client = OAuthClient {
        redirect_uri: format!("{}/api/auth/atproto/callback", public_url),
        client_id: format!("{}/api/auth/atproto/v2/client-metadata.json", public_url),
        private_signing_key_data: state.atproto.signing_key.clone(),
    };

    // Exchange authorization code for tokens
    let token_response = match oauth_complete(
        &http_client,
        &oauth_client,
        &pending.dpop_key,
        &params.code,
        &pending.oauth_request,
        &pending.auth_server,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "Token exchange failed");
            return (StatusCode::BAD_GATEWAY, "Token exchange failed").into_response();
        }
    };

    info!(
        scope = %token_response.scope,
        token_type = %token_response.token_type,
        expires_in = token_response.expires_in,
        "AT Protocol token exchange complete"
    );

    // The DID is in token_response.sub
    let did = match &token_response.sub {
        Some(sub) => sub.clone(),
        None => {
            error!("Token response missing sub (DID)");
            return (StatusCode::BAD_GATEWAY, "Identity verification failed").into_response();
        }
    };

    // Fetch public profile for display name and avatar
    let (display_name, avatar_url) = fetch_bsky_profile(&http_client, &did).await;
    // Use Bluesky handle as username (permanent DID is the user_id)
    let username = pending.handle.clone();

    // Find or create user — DID is the user_id
    let user_id = match users::find_by_oauth(&state.db, "atproto", &did).await {
        Ok(Some((uid, _))) => {
            // Update username to current handle (handles can change, DIDs are permanent)
            if let Err(e) = users::update_username(&state.db, &uid, &username).await {
                warn!(error = %e, "Failed to update username/handle");
            }
            uid
        }
        Ok(None) => {
            let oauth_id = Uuid::new_v4().to_string();
            if let Err(e) = users::create_with_oauth(
                &state.db,
                &users::CreateOAuthUser {
                    user_id: &did,
                    username: &username,
                    email: None,
                    avatar_url: avatar_url.as_deref(),
                    oauth_id: &oauth_id,
                    provider: "atproto",
                    provider_id: &did,
                },
            )
            .await
            {
                error!(error = %e, "Failed to create user");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create user")
                    .into_response();
            }
            info!(user_id = %did, username = %username, "new user registered via Bluesky");
            did.clone()
        }
        Err(e) => {
            error!(error = %e, "Database error during OAuth");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    // Store AT Protocol credentials for PDS API access (blob uploads, etc.)
    // Serialize the DPoP private key as JWK JSON to preserve the private key material.
    let dpop_key_str = match jwk::generate(&pending.dpop_key) {
        Ok(wrapped) => serde_json::to_string(&wrapped).unwrap_or_default(),
        Err(e) => {
            warn!(error = %e, "Failed to serialize DPoP key as JWK");
            String::new()
        }
    };
    let expires_at =
        (Utc::now() + chrono::Duration::seconds(token_response.expires_in as i64)).to_rfc3339();
    if let Err(e) = users::store_atproto_credentials(
        &state.db,
        &user_id,
        &token_response.access_token,
        token_response.refresh_token.as_deref().unwrap_or(""),
        &dpop_key_str,
        &pending.pds_url,
        &expires_at,
    )
    .await
    {
        warn!(error = %e, "Failed to store AT Protocol credentials (non-fatal)");
    }

    // Store handle on every login (ensures bsky_handle is populated for profile sync)
    let _ = crate::db::queries::atproto::store_bsky_profile_sync(
        &crate::db::queries::atproto::StoreBskyProfileParams {
            pool: &state.db,
            user_id: &user_id,
            handle: &username,
            display_name: display_name.as_deref(),
            description: None,
            banner_url: None,
            followers_count: 0,
            follows_count: 0,
        },
    )
    .await;

    // Issue session cookie and redirect
    issue_session_cookie(&state.auth_config, &user_id)
}

/// Resolve a Bluesky handle to the PDS URL.
async fn resolve_handle_to_pds(
    http_client: &reqwest::Client,
    handle: &str,
) -> Result<String, String> {
    // Resolve handle -> DID
    let did = resolve_handle(http_client, handle).await?;

    // Resolve DID -> DID document
    let did_doc = resolve_did_to_doc(http_client, &did).await?;

    // Extract PDS URL from the DID document
    let pds_endpoints = did_doc.pds_endpoints();
    let pds_url = pds_endpoints
        .first()
        .ok_or_else(|| "No PDS endpoint found in DID document".to_string())?;

    Ok(pds_url.to_string())
}

/// Resolve a handle to a DID. Tries the .well-known method first, then falls
/// back to the public Bluesky XRPC API (works for all handles including
/// custom domains and did:web identities).
pub async fn resolve_handle(http_client: &reqwest::Client, handle: &str) -> Result<String, String> {
    // Try .well-known/atproto-did first (works for self-hosted PDS)
    if let Ok(did) = atproto_identity::resolve::resolve_handle_http(http_client, handle).await {
        return Ok(did);
    }

    // Fallback: use the public Bluesky API
    #[derive(Deserialize)]
    struct ResolveResponse {
        did: String,
    }

    let url = format!(
        "https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle?handle={}",
        urlencoding::encode(handle)
    );

    let resp = http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Handle resolution failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Handle resolution failed: API returned {}",
            resp.status()
        ));
    }

    let resolved: ResolveResponse = resp
        .json()
        .await
        .map_err(|e| format!("Handle resolution failed: invalid response: {}", e))?;

    Ok(resolved.did)
}

/// Resolve a DID to its DID document.
pub async fn resolve_did_to_doc(
    http_client: &reqwest::Client,
    did: &str,
) -> Result<atproto_identity::model::Document, String> {
    if did.starts_with("did:plc:") {
        atproto_identity::plc::query(http_client, "plc.directory", did)
            .await
            .map_err(|e| format!("PLC DID resolution failed: {}", e))
    } else if did.starts_with("did:web:") {
        atproto_identity::web::query(http_client, did)
            .await
            .map_err(|e| format!("Web DID resolution failed: {}", e))
    } else {
        Err(format!("Unsupported DID method: {}", did))
    }
}

/// Fetch public Bluesky profile for display name and avatar.
async fn fetch_bsky_profile(
    http_client: &reqwest::Client,
    did: &str,
) -> (Option<String>, Option<String>) {
    #[derive(Deserialize)]
    struct BskyProfile {
        #[serde(rename = "displayName")]
        display_name: Option<String>,
        avatar: Option<String>,
        handle: Option<String>,
    }

    let url = format!(
        "https://public.api.bsky.app/xrpc/app.bsky.actor.getProfile?actor={}",
        urlencoding::encode(did)
    );

    match http_client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<BskyProfile>().await {
            Ok(profile) => {
                let name = profile
                    .display_name
                    .filter(|n| !n.is_empty())
                    .or(profile.handle);
                (name, profile.avatar)
            }
            Err(_) => (None, None),
        },
        _ => (None, None),
    }
}

/// Full Bluesky profile data returned by `fetch_full_bsky_profile()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyProfile {
    pub did: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub followers_count: i64,
    pub follows_count: i64,
    pub posts_count: i64,
}

/// Fetch a full public Bluesky profile via `app.bsky.actor.getProfile`.
/// Returns `None` if the profile cannot be fetched.
pub async fn fetch_full_bsky_profile(
    http_client: &reqwest::Client,
    did: &str,
) -> Option<BlueskyProfile> {
    #[derive(Deserialize)]
    struct RawProfile {
        did: String,
        handle: String,
        #[serde(rename = "displayName")]
        display_name: Option<String>,
        description: Option<String>,
        avatar: Option<String>,
        banner: Option<String>,
        #[serde(rename = "followersCount", default)]
        followers_count: i64,
        #[serde(rename = "followsCount", default)]
        follows_count: i64,
        #[serde(rename = "postsCount", default)]
        posts_count: i64,
    }

    let url = format!(
        "https://public.api.bsky.app/xrpc/app.bsky.actor.getProfile?actor={}",
        urlencoding::encode(did)
    );

    let resp = http_client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let raw: RawProfile = resp.json().await.ok()?;
    Some(BlueskyProfile {
        did: raw.did,
        handle: raw.handle,
        display_name: raw.display_name,
        description: raw.description,
        avatar: raw.avatar,
        banner: raw.banner,
        followers_count: raw.followers_count,
        follows_count: raw.follows_count,
        posts_count: raw.posts_count,
    })
}

/// Create a JWT and set it as an HttpOnly cookie, then redirect to app root.
fn issue_session_cookie(auth_config: &AuthConfig, user_id: &str) -> Response {
    let jwt = match create_session_token(
        user_id,
        &auth_config.jwt_secret,
        auth_config.session_expiry_hours,
    ) {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "Failed to create JWT");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Session creation failed").into_response();
        }
    };

    let secure = if auth_config.public_url.starts_with("https") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "concord_session={}; HttpOnly; Path=/; Max-Age={}; SameSite=Lax{}",
        jwt,
        auth_config.session_expiry_hours * 3600,
        secure,
    );

    (
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::temporary("/"),
    )
        .into_response()
}
