use super::{
    Actor, Aead, CURSOR_LIFETIME_SECONDS, ConversationId, CursorClaims, Engine, MAX_CURSOR_BYTES,
    PROTOCOL_VERSION, ReplayError, ReplayService, ResyncReason, Rng, Utc, XNonce,
    subscription_hash,
};

impl ReplayService {
    pub(super) fn encode_cursor(
        &self,
        actor: &Actor,
        subscriptions: &[ConversationId],
        generation: &str,
        event_sequence: i64,
    ) -> Result<String, ReplayError> {
        let now = Utc::now().timestamp();
        let expires_at = actor
            .expires_at()
            .unwrap_or(now + CURSOR_LIFETIME_SECONDS)
            .min(now + CURSOR_LIFETIME_SECONDS);
        let claims = CursorClaims {
            protocol_version: PROTOCOL_VERSION,
            database_generation: generation.to_owned(),
            principal_id: actor.user_id().as_str().to_owned(),
            credential_id: actor.credential_id().as_str().to_owned(),
            credential_version: actor.credential_version(),
            subscription_hash: subscription_hash(subscriptions),
            event_sequence,
            expires_at,
        };
        let plaintext = serde_json::to_vec(&claims).map_err(|_| ReplayError::InvalidInput)?;
        let mut nonce = [0_u8; 24];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .cursor_cipher
            .encrypt(&XNonce::from(nonce), plaintext.as_ref())
            .map_err(|_| ReplayError::InvalidInput)?;
        let mut encoded = nonce.to_vec();
        encoded.extend_from_slice(&ciphertext);
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(encoded))
    }

    pub(super) fn decode_cursor(&self, cursor: &str) -> Result<CursorClaims, ReplayError> {
        if cursor.len() > MAX_CURSOR_BYTES {
            return Err(ReplayError::ResyncRequired(ResyncReason::InvalidCursor));
        }
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?;
        if encoded.len() <= 24 {
            return Err(ReplayError::ResyncRequired(ResyncReason::InvalidCursor));
        }
        let (nonce, ciphertext) = encoded.split_at(24);
        let plaintext = self
            .cursor_cipher
            .decrypt(
                <&XNonce>::try_from(nonce)
                    .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?,
                ciphertext,
            )
            .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?;
        let claims: CursorClaims = serde_json::from_slice(&plaintext)
            .map_err(|_| ReplayError::ResyncRequired(ResyncReason::InvalidCursor))?;
        if claims.expires_at <= Utc::now().timestamp() {
            return Err(ReplayError::ResyncRequired(ResyncReason::CursorExpired));
        }
        Ok(claims)
    }

    pub(super) fn validate_cursor_actor(
        &self,
        claims: &CursorClaims,
        actor: &Actor,
        subscriptions: &[ConversationId],
    ) -> Result<(), ReplayError> {
        if claims.protocol_version != PROTOCOL_VERSION {
            return Err(ReplayError::ResyncRequired(ResyncReason::ProtocolChanged));
        }
        if claims.principal_id != actor.user_id().as_str()
            || claims.credential_id != actor.credential_id().as_str()
            || claims.credential_version != actor.credential_version()
        {
            return Err(ReplayError::ResyncRequired(ResyncReason::CredentialChanged));
        }
        if claims.subscription_hash != subscription_hash(subscriptions) {
            return Err(ReplayError::ResyncRequired(
                ResyncReason::SubscriptionChanged,
            ));
        }
        Ok(())
    }
}
