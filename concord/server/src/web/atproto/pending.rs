use super::{
    AuthorizationServer, Deserialize, MAX_PENDING_OAUTH, OAuthRequest, PendingAtprotoAuth,
    Serialize, Utc, jwk,
};

#[derive(Serialize, Deserialize)]
pub(super) struct EncryptedSigningKey {
    pub(super) key_id: String,
    pub(super) ciphertext: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PersistedPendingAtprotoAuth {
    pub(super) oauth_state: String,
    pub(super) issuer: String,
    pub(super) authorization_server: String,
    pub(super) nonce: String,
    pub(super) pkce_verifier: String,
    pub(super) signing_public_key: String,
    pub(super) dpop_private_key: String,
    pub(super) oauth_created_at: String,
    pub(super) oauth_expires_at: String,
    pub(super) handle: String,
    pub(super) auth_server: serde_json::Value,
    pub(super) pds_url: String,
    pub(super) resolved_did: String,
    pub(super) created_at: String,
}

pub(super) fn authorization_server_json(server: &AuthorizationServer) -> serde_json::Value {
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

pub(super) fn pending_state_hash(state: &str) -> String {
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

pub(super) async fn store_pending_oauth(
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

pub(super) async fn take_pending_oauth(
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
