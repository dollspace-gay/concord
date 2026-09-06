use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::http::header::SET_COOKIE;
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

use atproto_identity::key::{KeyData, KeyType, generate_key, to_public};
use atproto_oauth::jwk;
use atproto_oauth::pkce;
use atproto_oauth::resources::AuthorizationServer;
use atproto_oauth::workflow::{
    OAuthClient, OAuthRequest, OAuthRequestState, ParResponse, TokenResponse,
};
use atproto_oauth::{
    dpop::auth_dpop,
    jwt::{self, Claims, Header, JoseClaims},
};

use super::app_state::AppState;
use crate::db::queries::users;

/// State for pending AT Protocol OAuth flows.
pub struct AtprotoOAuth {
    /// ES256 private signing key for client assertions.
    pub signing_key: Option<KeyData>,
    /// Public JWK for the client metadata document.
    pub public_jwk: Option<jwk::WrappedJsonWebKey>,
}

pub struct PendingAtprotoAuth {
    pub oauth_request: OAuthRequest,
    pub dpop_key: KeyData,
    pub handle: String,
    pub auth_server: AuthorizationServer,
    pub pds_url: String,
    pub resolved_did: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Maximum number of pending OAuth flows at any time.
const MAX_PENDING_OAUTH: usize = 1000;
/// TTL for pending OAuth flows (10 minutes).
const PENDING_OAUTH_TTL_SECS: i64 = 600;

impl AtprotoOAuth {
    /// Load the signing key from the database, or generate and persist a new one.
    pub async fn load_or_create(
        pool: &sqlx::SqlitePool,
        vault: &crate::secrets::SecretVault,
    ) -> anyhow::Result<Self> {
        const KEY_NAME: &str = "atproto_signing_key";
        const CONTEXT: &str = "atproto:client-signing-key";

        // Try to load existing key from server_config
        let existing: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_config WHERE key = ?")
                .bind(KEY_NAME)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

        let signing_key = if let Some(ref stored) = existing {
            let jwk_json = if let Ok(envelope) = serde_json::from_str::<EncryptedSigningKey>(stored)
            {
                let plaintext = vault.decrypt(CONTEXT, &envelope.ciphertext, &envelope.key_id)?;
                String::from_utf8(plaintext)
                    .map_err(|_| anyhow::anyhow!("stored AT signing key is corrupt"))?
            } else {
                Self::store_encrypted(pool, KEY_NAME, CONTEXT, vault, stored).await?;
                stored.clone()
            };
            let wrapped = serde_json::from_str::<jwk::WrappedJsonWebKey>(&jwk_json)
                .map_err(|_| anyhow::anyhow!("stored AT signing key is corrupt"))?;
            jwk::to_key_data(&wrapped)
                .map_err(|_| anyhow::anyhow!("stored AT signing key is corrupt"))?
        } else {
            info!("no persisted AT Protocol signing key found, generating new one");
            Self::generate_and_store(pool, KEY_NAME, CONTEXT, vault).await?
        };

        let public_key =
            to_public(&signing_key).expect("failed to derive public key from signing key");
        let public_jwk =
            jwk::generate(&public_key).expect("failed to generate JWK from public key");
        Ok(Self {
            signing_key: Some(signing_key),
            public_jwk: Some(public_jwk),
        })
    }

    pub fn unavailable() -> Self {
        Self {
            signing_key: None,
            public_jwk: None,
        }
    }

    async fn generate_and_store(
        pool: &sqlx::SqlitePool,
        key_name: &str,
        context: &str,
        vault: &crate::secrets::SecretVault,
    ) -> anyhow::Result<KeyData> {
        let signing_key = generate_key(KeyType::P256Private)
            .map_err(|_| anyhow::anyhow!("failed to generate AT signing key"))?;
        let wrapped = jwk::generate(&signing_key)
            .map_err(|_| anyhow::anyhow!("failed to serialize AT signing key"))?;
        let jwk_json = serde_json::to_string(&wrapped)?;
        Self::store_encrypted(pool, key_name, context, vault, &jwk_json).await?;
        Ok(signing_key)
    }

    async fn store_encrypted(
        pool: &sqlx::SqlitePool,
        key_name: &str,
        context: &str,
        vault: &crate::secrets::SecretVault,
        plaintext: &str,
    ) -> anyhow::Result<()> {
        let envelope = EncryptedSigningKey {
            key_id: vault.key_id().into(),
            ciphertext: vault.encrypt(context, plaintext.as_bytes())?,
        };
        sqlx::query("INSERT INTO server_config(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=datetime('now')")
            .bind(key_name)
            .bind(serde_json::to_string(&envelope)?)
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct EncryptedSigningKey {
    key_id: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct PersistedPendingAtprotoAuth {
    oauth_state: String,
    issuer: String,
    authorization_server: String,
    nonce: String,
    pkce_verifier: String,
    signing_public_key: String,
    dpop_private_key: String,
    oauth_created_at: String,
    oauth_expires_at: String,
    handle: String,
    auth_server: serde_json::Value,
    pds_url: String,
    resolved_did: String,
    created_at: String,
}

fn authorization_server_json(server: &AuthorizationServer) -> serde_json::Value {
    serde_json::json!({
        "introspection_endpoint": server.introspection_endpoint,
        "authorization_endpoint": server.authorization_endpoint,
        "authorization_response_iss_parameter_supported": server.authorization_response_iss_parameter_supported,
        "client_id_metadata_document_supported": server.client_id_metadata_document_supported,
        "code_challenge_methods_supported": server.code_challenge_methods_supported,
        "dpop_signing_alg_values_supported": server.dpop_signing_alg_values_supported,
        "grant_types_supported": server.grant_types_supported,
        "issuer": server.issuer,
        "pushed_authorization_request_endpoint": server.pushed_authorization_request_endpoint,
        "request_parameter_supported": server.request_parameter_supported,
        "require_pushed_authorization_requests": server.require_pushed_authorization_requests,
        "response_types_supported": server.response_types_supported,
        "scopes_supported": server.scopes_supported,
        "token_endpoint_auth_methods_supported": server.token_endpoint_auth_methods_supported,
        "token_endpoint_auth_signing_alg_values_supported": server.token_endpoint_auth_signing_alg_values_supported,
        "token_endpoint": server.token_endpoint,
    })
}

fn pending_state_hash(state: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(state.as_bytes()))
}

impl PersistedPendingAtprotoAuth {
    fn from_pending(value: &PendingAtprotoAuth) -> Self {
        Self {
            oauth_state: value.oauth_request.oauth_state.clone(),
            issuer: value.oauth_request.issuer.clone(),
            authorization_server: value.oauth_request.authorization_server.clone(),
            nonce: value.oauth_request.nonce.clone(),
            pkce_verifier: value.oauth_request.pkce_verifier.clone(),
            signing_public_key: value.oauth_request.signing_public_key.clone(),
            dpop_private_key: value.oauth_request.dpop_private_key.clone(),
            oauth_created_at: value.oauth_request.created_at.to_rfc3339(),
            oauth_expires_at: value.oauth_request.expires_at.to_rfc3339(),
            handle: value.handle.clone(),
            auth_server: authorization_server_json(&value.auth_server),
            pds_url: value.pds_url.clone(),
            resolved_did: value.resolved_did.clone(),
            created_at: value.created_at.to_rfc3339(),
        }
    }

    fn into_pending(self) -> anyhow::Result<PendingAtprotoAuth> {
        let wrapped: jwk::WrappedJsonWebKey = serde_json::from_str(&self.dpop_private_key)
            .map_err(|_| anyhow::anyhow!("pending OAuth key is corrupt"))?;
        let dpop_key = jwk::to_key_data(&wrapped)
            .map_err(|_| anyhow::anyhow!("pending OAuth key is corrupt"))?;
        let parse_time = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| anyhow::anyhow!("pending OAuth timestamp is corrupt"))
        };
        Ok(PendingAtprotoAuth {
            oauth_request: OAuthRequest {
                oauth_state: self.oauth_state,
                issuer: self.issuer,
                authorization_server: self.authorization_server,
                nonce: self.nonce,
                pkce_verifier: self.pkce_verifier,
                signing_public_key: self.signing_public_key,
                dpop_private_key: self.dpop_private_key,
                created_at: parse_time(&self.oauth_created_at)?,
                expires_at: parse_time(&self.oauth_expires_at)?,
            },
            dpop_key,
            handle: self.handle,
            auth_server: serde_json::from_value(self.auth_server)
                .map_err(|_| anyhow::anyhow!("pending OAuth metadata is corrupt"))?,
            pds_url: self.pds_url,
            resolved_did: self.resolved_did,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

async fn store_pending_oauth(
    pool: &sqlx::SqlitePool,
    vault: &crate::secrets::SecretVault,
    pending: &PendingAtprotoAuth,
) -> anyhow::Result<()> {
    let state_hash = pending_state_hash(&pending.oauth_request.oauth_state);
    let context = format!("atproto:pending:{state_hash}");
    let plaintext = serde_json::to_vec(&PersistedPendingAtprotoAuth::from_pending(pending))?;
    let ciphertext = vault.encrypt(&context, &plaintext)?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("UPDATE pending_atproto_oauth SET state='expired',safe_error_code='expired' WHERE state='pending' AND expires_at<=datetime('now')")
        .execute(&mut *tx).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pending_atproto_oauth WHERE state='pending'")
            .fetch_one(&mut *tx)
            .await?;
    if count >= MAX_PENDING_OAUTH as i64 {
        return Err(anyhow::anyhow!("pending OAuth capacity exceeded"));
    }
    sqlx::query("INSERT INTO pending_atproto_oauth(state_hash,credential_key_id,credential_ciphertext,created_at,expires_at) VALUES(?,?,?,?,?)")
        .bind(&state_hash).bind(vault.key_id()).bind(ciphertext).bind(pending.created_at.to_rfc3339()).bind(pending.oauth_request.expires_at.to_rfc3339()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn take_pending_oauth(
    pool: &sqlx::SqlitePool,
    vault: &crate::secrets::SecretVault,
    state: &str,
) -> anyhow::Result<Option<PendingAtprotoAuth>> {
    let state_hash = pending_state_hash(state);
    let row:Option<(String,String)>=sqlx::query_as("UPDATE pending_atproto_oauth SET state='consumed',consumed_at=datetime('now') WHERE state_hash=? AND state='pending' AND expires_at>datetime('now') RETURNING credential_key_id,credential_ciphertext")
        .bind(&state_hash).fetch_optional(pool).await?;
    let Some((key_id, ciphertext)) = row else {
        return Ok(None);
    };
    let context = format!("atproto:pending:{state_hash}");
    let plaintext = vault.decrypt(&context, &ciphertext, &key_id)?;
    let persisted: PersistedPendingAtprotoAuth = serde_json::from_slice(&plaintext)
        .map_err(|_| anyhow::anyhow!("pending OAuth state is corrupt"))?;
    if persisted.oauth_state != state {
        return Err(anyhow::anyhow!("pending OAuth state is corrupt"));
    }
    Ok(Some(persisted.into_pending()?))
}

/// GET /api/auth/atproto/client-metadata.json — serves OAuth client metadata document.
pub async fn client_metadata(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let public_url = &state.auth_config.public_url;
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);

    let Some(public_jwk) = state.atproto.public_jwk.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AT Protocol login is unavailable",
        )
            .into_response();
    };
    let public_jwk_value =
        serde_json::to_value(public_jwk).expect("failed to serialize public JWK");

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

    Json(metadata).into_response()
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
    let Some(signing_key) = state.atproto.signing_key.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AT Protocol login is unavailable",
        )
            .into_response();
    };
    let client_id = format!("{}/api/auth/atproto/v2/client-metadata.json", public_url);
    let redirect_uri = format!("{}/api/auth/atproto/callback", public_url);

    // Resolve handle -> DID -> DID document -> PDS endpoint
    let (resolved_did, pds_url) = match resolve_handle_to_pds(&state.egress.oauth, &handle).await {
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
    let auth_server = match discover_authorization_server(&state.egress.oauth, &pds_url).await {
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
        private_signing_key_data: signing_key.clone(),
    };

    let request_state = OAuthRequestState {
        state: oauth_state.clone(),
        nonce: nonce.clone(),
        code_challenge,
        scope: "atproto transition:generic".to_string(),
    };

    // Make Pushed Authorization Request (PAR)
    let par_response = match oauth_init_controlled(
        &state.egress.oauth,
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

    let signing_pub = match to_public(&signing_key) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AT Protocol login key is unavailable",
            )
                .into_response();
        }
    };
    let signing_pub_jwk = match jwk::generate(&signing_pub) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AT Protocol login key is unavailable",
            )
                .into_response();
        }
    };
    let signing_public_key = match serde_json::to_string(&signing_pub_jwk) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AT Protocol login key is unavailable",
            )
                .into_response();
        }
    };

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

    let pending = PendingAtprotoAuth {
        oauth_request,
        dpop_key,
        handle: handle.clone(),
        auth_server: auth_server.clone(),
        pds_url: pds_url.clone(),
        resolved_did,
        created_at: Utc::now(),
    };
    if let Err(error) = store_pending_oauth(&state.db, &state.secret_vault, &pending).await {
        warn!(error=%error,"Failed to persist pending OAuth request");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth login state could not be stored",
        )
            .into_response();
    }

    // Redirect user to authorization server
    let auth_url = format!(
        "{}?client_id={}&request_uri={}",
        auth_server.authorization_endpoint,
        urlencoding::encode(&oauth_client.client_id),
        urlencoding::encode(&par_response.request_uri),
    );

    let mut response = Redirect::temporary(&auth_url).into_response();
    if let Ok(value)=format!(
        "concord_at_state={oauth_state}; HttpOnly; Secure; Path=/api/auth/atproto/callback; Max-Age={PENDING_OAUTH_TTL_SECS}; SameSite=Lax"
    ).parse() {
        response.headers_mut().append(SET_COOKIE,value);
    }
    response
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
    jar: CookieJar,
    Query(params): Query<AtprotoCallbackParams>,
) -> Response {
    if jar.get("concord_at_state").map(|cookie| cookie.value()) != Some(params.state.as_str()) {
        return (StatusCode::BAD_REQUEST, "OAuth browser state mismatch").into_response();
    }
    // Look up pending request
    let pending = match take_pending_oauth(&state.db, &state.secret_vault, &params.state).await {
        Ok(value) => value,
        Err(error) => {
            error!(error=%error,"Pending OAuth recovery failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "OAuth login state is unavailable",
            )
                .into_response();
        }
    };
    let Some(pending) = pending else {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid or expired state parameter",
        )
            .into_response();
    };
    if Utc::now() > pending.oauth_request.expires_at
        || Utc::now() - pending.created_at > chrono::Duration::seconds(PENDING_OAUTH_TTL_SECS)
    {
        return (StatusCode::BAD_REQUEST, "OAuth request expired").into_response();
    }

    // Verify issuer matches if provided
    if let Some(ref iss) = params.iss
        && *iss != pending.oauth_request.issuer
    {
        return (StatusCode::BAD_REQUEST, "Issuer mismatch").into_response();
    }

    let public_url = &state.auth_config.public_url;
    let Some(signing_key) = state.atproto.signing_key.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AT Protocol login is unavailable",
        )
            .into_response();
    };

    let oauth_client = OAuthClient {
        redirect_uri: format!("{}/api/auth/atproto/callback", public_url),
        client_id: format!("{}/api/auth/atproto/v2/client-metadata.json", public_url),
        private_signing_key_data: signing_key,
    };

    // Exchange authorization code for tokens
    let token_response = match oauth_complete_controlled(
        &state.egress.oauth,
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
    if token_response.sub.as_deref() != Some(pending.resolved_did.as_str()) {
        return (StatusCode::BAD_GATEWAY, "Provider subject mismatch").into_response();
    }

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
    let (display_name, avatar_url) = fetch_bsky_profile(&state.egress.oauth, &did).await;
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
    if let Err(e) = users::store_atproto_credentials_encrypted(
        &state.db,
        &state.secret_vault,
        &user_id,
        &users::AtprotoCredentials {
            did: did.clone(),
            access_token: token_response.access_token.clone(),
            refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
            dpop_private_key: dpop_key_str,
            pds_url: pending.pds_url.clone(),
            authorization_issuer: pending.auth_server.issuer.clone(),
            token_endpoint: pending.auth_server.token_endpoint.clone(),
            token_expires_at: expires_at,
            credential_version: 0,
        },
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

    if let Err(error) =
        crate::config::ensure_configured_admin(&state.db, &user_id, &state.admin_user_ids).await
    {
        error!(%error, "failed to apply stable administrator bootstrap");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Administrator bootstrap could not be verified",
        )
            .into_response();
    }

    // Issue session cookie and redirect
    issue_session_cookie(state.as_ref(), &user_id).await
}

async fn provider_get_json<T: serde::de::DeserializeOwned>(
    transport: &crate::egress::ControlledHttpClient,
    url: reqwest::Url,
) -> anyhow::Result<T> {
    let request = transport
        .request(
            reqwest::Method::GET,
            url,
            crate::egress::RedirectPolicy::FollowSafeGet,
        )?
        .header(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    let response = transport.send(request).await?;
    if !response.status.is_success() {
        anyhow::bail!("provider returned status {}", response.status)
    }
    Ok(crate::egress::parse_provider_json(&response.body)?)
}

#[derive(Deserialize)]
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
}

async fn discover_authorization_server(
    transport: &crate::egress::ControlledHttpClient,
    pds: &str,
) -> anyhow::Result<AuthorizationServer> {
    let pds_url = reqwest::Url::parse(pds)?;
    let resource_url = pds_url.join("/.well-known/oauth-protected-resource")?;
    let resource: ResourceMetadata = provider_get_json(transport, resource_url).await?;
    if resource.resource.trim_end_matches('/') != pds_url.as_str().trim_end_matches('/')
        || resource.authorization_servers.len() != 1
    {
        anyhow::bail!("invalid protected resource metadata")
    }
    let issuer = reqwest::Url::parse(&resource.authorization_servers[0])?;
    let metadata_url = issuer.join("/.well-known/oauth-authorization-server")?;
    let metadata: AuthorizationServer = provider_get_json(transport, metadata_url).await?;
    if metadata.issuer.trim_end_matches('/') != issuer.as_str().trim_end_matches('/')
        || !metadata.authorization_response_iss_parameter_supported
        || !metadata.client_id_metadata_document_supported
        || !metadata
            .code_challenge_methods_supported
            .iter()
            .any(|v| v == "S256")
        || !metadata
            .dpop_signing_alg_values_supported
            .iter()
            .any(|v| v == "ES256")
        || !metadata
            .grant_types_supported
            .iter()
            .any(|v| v == "authorization_code")
        || !metadata
            .grant_types_supported
            .iter()
            .any(|v| v == "refresh_token")
        || !metadata.require_pushed_authorization_requests
        || !metadata
            .response_types_supported
            .iter()
            .any(|v| v == "code")
        || !metadata
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|v| v == "private_key_jwt")
        || !metadata
            .token_endpoint_auth_signing_alg_values_supported
            .iter()
            .any(|v| v == "ES256")
    {
        anyhow::bail!("authorization server metadata failed validation")
    }
    for endpoint in [
        &metadata.authorization_endpoint,
        &metadata.pushed_authorization_request_endpoint,
        &metadata.token_endpoint,
    ] {
        let url = reqwest::Url::parse(endpoint)?;
        if url.origin() != issuer.origin() {
            anyhow::bail!("authorization endpoint origin mismatch")
        }
    }
    Ok(metadata)
}

fn client_assertion(client: &OAuthClient, audience: &str) -> anyhow::Result<String> {
    let header: Header = client
        .private_signing_key_data
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("client assertion header failed"))?;
    let claims = Claims::new(JoseClaims {
        issuer: Some(client.client_id.clone()),
        subject: Some(client.client_id.clone()),
        audience: Some(audience.into()),
        json_web_token_id: Some(Uuid::new_v4().to_string()),
        issued_at: Some(Utc::now().timestamp() as u64),
        ..Default::default()
    });
    jwt::mint(&client.private_signing_key_data, &header, &claims)
        .map_err(|_| anyhow::anyhow!("client assertion failed"))
}

async fn oauth_form_post(
    transport: &crate::egress::ControlledHttpClient,
    dpop_key: &KeyData,
    url: &str,
    form: &str,
) -> anyhow::Result<crate::egress::EgressResponse> {
    let url = reqwest::Url::parse(url)?;
    let (proof, _, _) = auth_dpop(dpop_key, "POST", url.as_str())
        .map_err(|_| anyhow::anyhow!("DPoP proof failed"))?;
    let mut response = oauth_form_post_once(transport, &url, &proof, form).await?;
    if matches!(
        response.status,
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
    ) && let Some(nonce) = response
        .headers
        .get("DPoP-Nonce")
        .and_then(|v| v.to_str().ok())
    {
        let (_, header, mut claims) = auth_dpop(dpop_key, "POST", url.as_str())
            .map_err(|_| anyhow::anyhow!("DPoP nonce proof failed"))?;
        claims
            .private
            .insert("nonce".into(), nonce.to_string().into());
        let proof = jwt::mint(dpop_key, &header, &claims)
            .map_err(|_| anyhow::anyhow!("DPoP nonce proof failed"))?;
        response = oauth_form_post_once(transport, &url, &proof, form).await?;
    }
    Ok(response)
}
async fn oauth_form_post_once(
    transport: &crate::egress::ControlledHttpClient,
    url: &reqwest::Url,
    proof: &str,
    form: &str,
) -> anyhow::Result<crate::egress::EgressResponse> {
    let request = transport
        .request(
            reqwest::Method::POST,
            url.clone(),
            crate::egress::RedirectPolicy::Reject,
        )?
        .header(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .header(
            reqwest::header::HeaderName::from_static("dpop"),
            reqwest::header::HeaderValue::from_str(proof)?,
        )
        .body(form.as_bytes().to_vec())
        .credentials_for(url)?;
    Ok(transport.send(request).await?)
}

async fn oauth_init_controlled(
    transport: &crate::egress::ControlledHttpClient,
    client: &OAuthClient,
    dpop_key: &KeyData,
    login_hint: Option<&str>,
    server: &AuthorizationServer,
    state: &OAuthRequestState,
) -> anyhow::Result<ParResponse> {
    let assertion = client_assertion(client, &server.issuer)?;
    let form = {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer
            .append_pair("response_type", "code")
            .append_pair("code_challenge", &state.code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("client_id", &client.client_id)
            .append_pair("state", &state.state)
            .append_pair("redirect_uri", &client.redirect_uri)
            .append_pair("scope", &state.scope)
            .append_pair(
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            )
            .append_pair("client_assertion", &assertion);
        if let Some(hint) = login_hint {
            serializer.append_pair("login_hint", hint);
        }
        serializer.finish()
    };
    let response = oauth_form_post(
        transport,
        dpop_key,
        &server.pushed_authorization_request_endpoint,
        &form,
    )
    .await?;
    if !response.status.is_success() {
        anyhow::bail!("PAR returned status {}", response.status)
    }
    Ok(crate::egress::parse_provider_json(&response.body)?)
}

async fn oauth_complete_controlled(
    transport: &crate::egress::ControlledHttpClient,
    client: &OAuthClient,
    dpop_key: &KeyData,
    code: &str,
    request: &OAuthRequest,
    server: &AuthorizationServer,
) -> anyhow::Result<TokenResponse> {
    let assertion = client_assertion(client, &server.issuer)?;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &client.redirect_uri)
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("code_verifier", &request.pkce_verifier)
        .append_pair(
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
        )
        .append_pair("client_assertion", &assertion)
        .finish();
    let response = oauth_form_post(transport, dpop_key, &server.token_endpoint, &form).await?;
    if !response.status.is_success() {
        anyhow::bail!("token endpoint returned status {}", response.status)
    }
    Ok(crate::egress::parse_provider_json(&response.body)?)
}

/// Resolve a verified handle through its DID document to a PDS URL.
async fn resolve_handle_to_pds(
    transport: &crate::egress::ControlledHttpClient,
    handle: &str,
) -> Result<(String, String), String> {
    let did = resolve_handle(transport, handle).await?;
    let document = resolve_did_to_doc(transport, &did).await?;
    let pds = document
        .pds_endpoints()
        .first()
        .ok_or_else(|| "No PDS endpoint found".to_string())?
        .to_string();
    Ok((did, pds))
}

async fn resolve_handle(
    transport: &crate::egress::ControlledHttpClient,
    handle: &str,
) -> Result<String, String> {
    if handle.is_empty()
        || handle.len() > 253
        || handle.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err("Invalid handle".into());
    }
    #[derive(Deserialize)]
    struct ResolveResponse {
        did: String,
    }
    let well_known = reqwest::Url::parse(&format!("https://{handle}/.well-known/atproto-did"))
        .map_err(|_| "Invalid handle".to_string())?;
    if let Ok(request) = transport
        .request(
            reqwest::Method::GET,
            well_known,
            crate::egress::RedirectPolicy::FollowSafeGet,
        )
        .map_err(anyhow::Error::from)
        && let Ok(response) = transport.send(request).await
        && response.status.is_success()
        && response.body.len() <= 2048
        && let Ok(did) = std::str::from_utf8(&response.body)
    {
        let did = did.trim();
        if did.starts_with("did:") {
            return Ok(did.into());
        }
    }
    let url = reqwest::Url::parse_with_params(
        "https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle",
        &[("handle", handle)],
    )
    .map_err(|_| "Invalid resolver URL".to_string())?;
    let resolved: ResolveResponse = provider_get_json(transport, url)
        .await
        .map_err(|_| "Handle resolution failed".to_string())?;
    if !resolved.did.starts_with("did:") {
        return Err("Resolver returned invalid DID".into());
    }
    Ok(resolved.did)
}

async fn resolve_did_to_doc(
    transport: &crate::egress::ControlledHttpClient,
    did: &str,
) -> Result<atproto_identity::model::Document, String> {
    let url = if let Some(identifier) = did.strip_prefix("did:plc:") {
        if identifier.is_empty() || !identifier.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("Invalid PLC DID".into());
        }
        reqwest::Url::parse(&format!("https://plc.directory/{did}"))
            .map_err(|_| "Invalid PLC DID".to_string())?
    } else if let Some(identifier) = did.strip_prefix("did:web:") {
        if identifier.contains('%') {
            return Err("Encoded did:web is unsupported".into());
        }
        let mut parts = identifier.split(':');
        let host = parts.next().ok_or("Invalid did:web")?;
        if host.is_empty() {
            return Err("Invalid did:web".into());
        }
        let path: Vec<&str> = parts.collect();
        let raw = if path.is_empty() {
            format!("https://{host}/.well-known/did.json")
        } else {
            format!("https://{host}/{}/did.json", path.join("/"))
        };
        reqwest::Url::parse(&raw).map_err(|_| "Invalid did:web".to_string())?
    } else {
        return Err("Unsupported DID method".into());
    };
    provider_get_json(transport, url)
        .await
        .map_err(|_| "DID resolution failed".into())
}

async fn fetch_bsky_profile(
    transport: &crate::egress::ControlledHttpClient,
    did: &str,
) -> (Option<String>, Option<String>) {
    #[derive(Deserialize)]
    struct Profile {
        #[serde(rename = "displayName")]
        display_name: Option<String>,
        avatar: Option<String>,
        handle: Option<String>,
    }
    let Ok(url) = reqwest::Url::parse_with_params(
        "https://public.api.bsky.app/xrpc/app.bsky.actor.getProfile",
        &[("actor", did)],
    ) else {
        return (None, None);
    };
    match provider_get_json::<Profile>(transport, url).await {
        Ok(profile) => (
            profile
                .display_name
                .filter(|name| !name.is_empty())
                .or(profile.handle),
            profile.avatar,
        ),
        Err(_) => (None, None),
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
    transport: &crate::egress::ControlledHttpClient,
    endpoint: &reqwest::Url,
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

    let mut url = endpoint.clone();
    url.query_pairs_mut().append_pair("actor", did);
    let raw: RawProfile = provider_get_json(transport, url).await.ok()?;
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
async fn issue_session_cookie(state: &AppState, user_id: &str) -> Response {
    let jwt = match state.auth.issue_web_session(user_id).await {
        Ok((token, _actor)) => token,
        Err(e) => {
            error!(error = %e, "Failed to create JWT");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Session creation failed").into_response();
        }
    };

    let secure = if state.auth_config.public_url.starts_with("https") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "concord_session={}; HttpOnly; Path=/; Max-Age={}; SameSite=Lax{}",
        jwt,
        state.auth_config.session_expiry_hours * 3600,
        secure,
    );

    (
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::temporary("/"),
    )
        .into_response()
}

#[cfg(test)]
mod signing_key_tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};

    fn vault(byte: u8) -> (tempfile::NamedTempFile, crate::secrets::SecretVault) {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), hex::encode([byte; 32])).unwrap();
        let vault = crate::secrets::SecretVault::load(file.path()).unwrap();
        (file, vault)
    }

    #[tokio::test]
    async fn signing_key_is_encrypted_stable_and_lost_key_fails() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let (_file, first) = vault(21);
        let generated = AtprotoOAuth::load_or_create(&pool, &first).await.unwrap();
        let expected = serde_json::to_value(&generated.public_jwk).unwrap();
        let stored: String =
            sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!stored.contains("\"d\":"));
        let loaded = AtprotoOAuth::load_or_create(&pool, &first).await.unwrap();
        assert_eq!(serde_json::to_value(&loaded.public_jwk).unwrap(), expected);
        let (_wrong_file, wrong) = vault(22);
        assert!(AtprotoOAuth::load_or_create(&pool, &wrong).await.is_err());
    }

    fn pending(state: &str) -> PendingAtprotoAuth {
        let dpop_key = generate_key(KeyType::P256Private).unwrap();
        let dpop_private_key = serde_json::to_string(&jwk::generate(&dpop_key).unwrap()).unwrap();
        let now = Utc::now();
        PendingAtprotoAuth {
            oauth_request: OAuthRequest {
                oauth_state: state.into(),
                issuer: "https://issuer.example".into(),
                authorization_server: "https://issuer.example".into(),
                nonce: "nonce".into(),
                pkce_verifier: "verifier".into(),
                signing_public_key: "public".into(),
                dpop_private_key,
                created_at: now,
                expires_at: now + chrono::Duration::minutes(5),
            },
            dpop_key,
            handle: "alice.example".into(),
            auth_server: AuthorizationServer {
                issuer: "https://issuer.example".into(),
                authorization_endpoint: "https://issuer.example/authorize".into(),
                token_endpoint: "https://issuer.example/token".into(),
                ..Default::default()
            },
            pds_url: "https://pds.example".into(),
            resolved_did: "did:plc:alice".into(),
            created_at: now,
        }
    }

    #[tokio::test]
    async fn pending_oauth_is_encrypted_durable_one_time_and_key_bound() {
        let directory = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", directory.path().join("oauth.db").display());
        let pool = create_pool(&url).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let (_file, first) = vault(31);
        store_pending_oauth(&pool, &first, &pending("state-one"))
            .await
            .unwrap();
        let stored: String =
            sqlx::query_scalar("SELECT credential_ciphertext FROM pending_atproto_oauth")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!stored.contains("verifier"));
        pool.close().await;
        let reopened = create_pool(&url).await.unwrap();
        let recovered = take_pending_oauth(&reopened, &first, "state-one")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.resolved_did, "did:plc:alice");
        assert!(
            take_pending_oauth(&reopened, &first, "state-one")
                .await
                .unwrap()
                .is_none()
        );

        store_pending_oauth(&reopened, &first, &pending("state-two"))
            .await
            .unwrap();
        let (_wrong_file, wrong) = vault(32);
        assert!(
            take_pending_oauth(&reopened, &wrong, "state-two")
                .await
                .is_err()
        );
        assert!(
            take_pending_oauth(&reopened, &first, "state-two")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn corrupt_pending_oauth_fails_closed_and_is_not_replayable() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let (_file, vault) = vault(41);
        store_pending_oauth(&pool, &vault, &pending("state-corrupt"))
            .await
            .unwrap();
        sqlx::query("UPDATE pending_atproto_oauth SET credential_ciphertext='corrupt'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            take_pending_oauth(&pool, &vault, "state-corrupt")
                .await
                .is_err()
        );
        assert!(
            take_pending_oauth(&pool, &vault, "state-corrupt")
                .await
                .unwrap()
                .is_none()
        );
    }
}
