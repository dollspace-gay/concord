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
    jwt::{Claims, Header, JoseClaims},
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

#[cfg(test)]
mod signing_key_tests;

mod callback;
mod discovery;
mod exchange;
mod login;
mod pending;
mod profiles;
mod resolution;
mod sessions;
pub use callback::AtprotoCallbackParams;
pub use callback::atproto_callback;
use discovery::discover_authorization_server;
use discovery::provider_get_json;
use exchange::oauth_complete_controlled;
use exchange::oauth_init_controlled;
pub use login::AtprotoLoginParams;
pub use login::atproto_login;
pub use login::client_metadata;
use pending::EncryptedSigningKey;
use pending::store_pending_oauth;
use pending::take_pending_oauth;
pub use profiles::BlueskyProfile;
use profiles::fetch_bsky_profile;
pub use profiles::fetch_full_bsky_profile;
use resolution::resolve_handle_to_pds;
use sessions::issue_session_cookie;
